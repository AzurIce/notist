#!/usr/bin/env python3
"""Hash every complete analysis/evaluation/prepared tree and observed dependency set.

Canonical JSON streams go straight to hashlib, not to an unbounded archive file.
These instrumented executions are diagnostics, never primary speed/RSS samples.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

import profile_runtime_cow as common
from profile_ten_rounds import OUT, REPO, guard


def prepare(name):
    source = OUT / 'sources' / ('semantic-' + name)
    shutil.copytree(OUT / 'sources' / name, source)
    path = source / 'crates/notist-ssg/src/build.rs'
    text = path.read_text()
    anchor = '        let evaluation = vault.evaluate_document(&handle, &analysis)?;\n'
    assert text.count(anchor) == 1
    text = text.replace(anchor, anchor + '        semantic_tree_probe::evaluation(&route.path, &analysis, &evaluation);\n')
    anchor = '        handles.push(handle);\n'
    assert text.count(anchor) == 1
    text = text.replace(anchor, '        semantic_tree_probe::prepared(documents.last().unwrap());\n' + anchor)
    path.write_text('#[path = "semantic_tree_probe.rs"]\nmod semantic_tree_probe;\n' + text)
    shutil.copy2(REPO / 'tools/semantic_tree_probe.rs', source / 'crates/notist-ssg/src/semantic_tree_probe.rs')
    path = source / 'crates/notist-cli/src/ssg.rs'
    text = path.read_text()
    anchor = '    let observed = vault.resources().observed();\n'
    assert text.count(anchor) == 1
    text = text.replace(anchor, anchor + '''    {
        use std::io::Write;
        let stderr = std::io::stderr();
        let mut out = std::io::BufWriter::new(stderr.lock());
        out.write_all(b"[semantic-page] [\\"observed\\",\\"build\\"]\\n[semantic-json] ").unwrap();
        serde_json::to_writer(&mut out, &(&observed, vault.input_session().dependencies())).unwrap();
        out.write_all(b"\\n").unwrap();
        out.flush().unwrap();
    }
''')
    path.write_text(text)
    common.save(OUT / ('semantic-' + name + '-sources.json'), common.inventory(source))


def build(name):
    source = OUT / 'sources' / ('semantic-' + name)
    env = dict(os.environ, CARGO_TARGET_DIR=str(OUT / 'target-probe'), CARGO_BUILD_JOBS='2',
               CARGO_INCREMENTAL='0', CARGO_PROFILE_RELEASE_DEBUG='1')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=source, env=env))
    packages = [p['name'] for p in metadata['packages'] if p['id'] in metadata['workspace_members']]
    with (OUT / 'logs' / ('build-semantic-' + name + '.log')).open('wb') as log:
        subprocess.run(['cargo', 'clean', '--release', *[flag for p in packages for flag in ['-p', p]]], cwd=source,
                       env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'notist-cli', '--bin', 'notist'],
                       cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(OUT / 'target-probe/release/notist', OUT / 'bin' / ('semantic-' + name))


def run(name):
    guard()
    label = 'semantic-' + name
    evidence = OUT / (label + '.jsonl')
    assert not evidence.exists()
    output = OUT / 'site/dist'
    if output.exists():
        shutil.rmtree(output)
    count, total = 0, 0
    with evidence.open('w') as result, (OUT / 'logs' / (label + '.stdout')).open('wb') as stdout, (OUT / 'logs' / (label + '.stderr')).open('wb') as stderr:
        child = subprocess.Popen([str(OUT / 'bin' / label), 'build'], cwd=OUT / 'site',
                                 env=dict(os.environ, NO_COLOR='1', TERM='dumb', PYTHONDONTWRITEBYTECODE='1'),
                                 stdout=stdout, stderr=subprocess.PIPE)
        try:
            for line in child.stderr:
                if not line.startswith(b'[semantic-page] '):
                    stderr.write(line)
                    continue
                kind, path = json.loads(line.removeprefix(b'[semantic-page] '))
                payload = child.stderr.readline()
                assert payload.startswith(b'[semantic-json] '), (kind, path)
                payload = payload.removeprefix(b'[semantic-json] ').removesuffix(b'\n')
                row = dict(kind=kind, path=path, bytes=len(payload), sha256=hashlib.sha256(payload).hexdigest())
                result.write(json.dumps(row, sort_keys=True, ensure_ascii=False) + '\n')
                count += 1
                total += len(payload)
            assert child.wait() == 0
        finally:
            if child.poll() is None:
                child.terminate()
                child.wait()
    assert count == 2 * 51_739 + 1, count
    assert common.inventory(OUT / 'site/dist/icexiv-db') == json.loads((OUT / 'output-inventory.json').read_text())
    common.save(OUT / (label + '-summary.json'), dict(frames=count, canonical_json_bytes=total,
                evidence_sha256=common.digest(evidence), binary_sha256=common.digest(OUT / 'bin' / label), output_verified=True))
    print(label, count, 'frames;', total, 'canonical bytes;', common.digest(evidence), flush=True)
    guard()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'build', 'run'])
    parser.add_argument('--name', default='r00')
    args = parser.parse_args()
    globals()[args.action](args.name)
