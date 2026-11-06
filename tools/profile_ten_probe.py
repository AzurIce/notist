#!/usr/bin/env python3
"""Separate reproducible diagnostic source; never uses its wall time for gains."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess

import profile_runtime_cow as common
from profile_ten_rounds import OUT, REPO, guard

LABEL = None


def prepare(name):
    original = OUT / 'sources' / name
    name = LABEL or name
    source = OUT / 'sources' / ('probe-' + name)
    shutil.copytree(original, source)
    path = source / 'crates/notist-core/src/shared.rs'
    text = path.read_text()
    index_count = 'self.0.index.as_ref().map_or(0, |index| index.len())' if 'index: Option<Box<BTreeMap<Text, usize>>>' in text else 'self.0.index.len()'
    anchor = 'impl Dict {\n'
    assert text.count(anchor) == 1
    text = text.replace(anchor, anchor + '''    pub fn residency_probe(&self) -> (usize, usize, usize, usize, usize) {
        (Arc::as_ptr(&self.0) as usize, self.0.entries.len(), self.0.entries.capacity(), INDEX_COUNT, std::mem::size_of::<DictStorage>())
    }
'''.replace('INDEX_COUNT', index_count))
    path.write_text(text)
    path = source / 'crates/notist-ssg/src/build.rs'
    text = path.read_text()
    anchor = '    evaluation_stage.finish();\n'
    assert text.count(anchor) == 1
    path.write_text('#[path = "tree_residency_probe.rs"]\nmod tree_residency_probe;\n' +
                    text.replace(anchor, anchor + '    tree_residency_probe::observe(&documents);\n'))
    shutil.copy2(REPO / 'tools/tree_residency_probe.rs', source / 'crates/notist-ssg/src/tree_residency_probe.rs')
    common.save(OUT / ('probe-' + name + '-sources.json'), common.inventory(source))


def build(name):
    name = LABEL or name
    source = OUT / 'sources' / ('probe-' + name)
    expected = json.loads((OUT / ('probe-' + name + '-sources.json')).read_text())
    assert common.inventory(source) == expected
    env = dict(os.environ, CARGO_TARGET_DIR=str(OUT / 'target-probe'), CARGO_BUILD_JOBS='2',
               CARGO_INCREMENTAL='0', CARGO_PROFILE_RELEASE_DEBUG='1')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=source, env=env))
    packages = [p['name'] for p in metadata['packages'] if p['id'] in metadata['workspace_members']]
    with (OUT / 'logs' / ('build-probe-' + name + '.log')).open('wb') as log:
        subprocess.run(['cargo', 'clean', '--release', *[flag for package in packages for flag in ['-p', package]]],
                       cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'notist-cli', '--bin', 'notist'],
                       cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    assert common.inventory(source) == expected
    shutil.copy2(OUT / 'target-probe/release/notist', OUT / 'bin' / ('probe-' + name))


def run(name):
    name = LABEL or name
    guard()
    label = 'probe-' + name
    output = OUT / 'site/dist'
    if output.exists():
        shutil.rmtree(output)
    with (OUT / 'logs' / (label + '.stdout')).open('wb') as stdout, (OUT / 'logs' / (label + '.stderr')).open('wb') as stderr:
        subprocess.run([str(OUT / 'bin' / label), 'build'], cwd=OUT / 'site',
                       env=dict(os.environ, NO_COLOR='1', TERM='dumb', PYTHONDONTWRITEBYTECODE='1'),
                       stdout=stdout, stderr=stderr, check=True)
    text = (OUT / 'logs' / (label + '.stderr')).read_text()
    points = [json.loads(line.removeprefix('[tree-residency] ')) for line in text.splitlines() if line.startswith('[tree-residency] ')]
    assert len(points) == 1
    assert common.inventory(OUT / 'site/dist/icexiv-db') == json.loads((OUT / 'output-inventory.json').read_text())
    common.save(OUT / (label + '.json'), dict(binary_sha256=common.digest(OUT / 'bin' / label),
                                           output_verified=True, snapshot=points[0]))
    print(json.dumps(points[0], indent=2), flush=True)
    guard()


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=['prepare', 'build', 'run'])
    p.add_argument('--name', default='r00')
    p.add_argument('--label', help='Separate diagnostic generation; never overwrite old evidence')
    args = p.parse_args()
    LABEL = args.label
    globals()[args.action](args.name)
