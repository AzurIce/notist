#!/usr/bin/env python3
"""After-only supplemental probes for publication checks, never speedup samples."""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess

from profile_optimizations import guard, process
from profile_resource_overhead import fingerprints
from profile_xiv_history import once, save, sha


def instrument_publication(text):
    head, body = text.split('pub fn publish(', 1)
    body = once(body, '    let output = normalize(',
                '    let resolve = notist::xiv_profile::span("publish.resolve");\n    let output = normalize(')
    body = once(body, '    if output.exists() {\n        let metadata =',
                '    drop(resolve);\n    let ownership = notist::xiv_profile::span("publish.ownership");\n    if output.exists() {\n        let metadata =')
    body = once(body, '    for relative in files.keys() {',
                '    drop(ownership);\n    let validation = notist::xiv_profile::span("publish.path_validation");\n    for relative in files.keys() {')
    body = once(body, '    if output.exists() && unchanged(files, &output) {',
                '    drop(validation);\n    if output.exists() && unchanged(files, &output) {')
    return head + 'pub fn publish(' + body


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--cpu', type=int, default=8)
    parser.add_argument('--report-only', action='store_true')
    args = parser.parse_args()
    out = args.out.resolve()
    manifest = guard(args)
    evidence = out / 'publication-checks'
    if args.report_only:
        summarize(out)
        return
    assert not evidence.exists()
    evidence.mkdir()
    source = evidence / 'source'
    shutil.copytree(out / 'sources/after-probe', source)
    path = source / 'crates/notist-cli/src/publication.rs'
    original = path.read_text()
    rewritten = instrument_publication(original)
    path.write_text(rewritten)
    env = dict(os.environ, **manifest['build_env'], NO_COLOR='1', TERM='dumb',
               CARGO_TARGET_DIR=str(out / 'target-after-probe'))
    command = ['cargo', 'build', '--locked', '--release', '-p', 'notist-cli',
               '--bin', 'notist', '--features', 'profiling']
    with (evidence / 'build.log').open('wb') as log:
        subprocess.run(command, cwd=source, env=env, stdout=log,
                       stderr=subprocess.STDOUT, check=True)
    binary = out / 'bin/publication-checks-probe'
    shutil.copy2(out / 'target-after-probe/release/notist', binary)
    proof = {'kind': 'after-only-supplement', 'base_patch_sha256': manifest['patch_sha256'],
             'binary_sha256': sha(binary.read_bytes()), 'sources': fingerprints(source),
             'rewrite': {'before_sha256': sha(original.encode()), 'after_sha256': sha(rewritten.encode())},
             'cpu': args.cpu, 'rounds': 3, 'warmups': 1,
             'not_used_for_before_after_speedups': True}
    save(evidence / 'manifest.json', proof)
    rows = []
    for workload, scenario in [('xiv', 'same'), ('private', 'same'), ('xiv', 'replace')]:
        for rep in range(-1, 3):
            label = f'publication-checks-{workload}-{scenario}-{rep}'
            row = process(args, 'publication-checks-probe', workload, scenario, label)
            row['round'] = rep
            rows.append(row)
            save(evidence / 'samples.json', rows)
            print(label, round(row['wall_s'], 4), flush=True)
    assert fingerprints(source) == proof['sources']
    assert sha(binary.read_bytes()) == proof['binary_sha256']
    guard(args)
    summarize(out)


def summarize(out):
    evidence = out / 'publication-checks'
    rows = json.loads((evidence / 'samples.json').read_text())
    proof = json.loads((evidence / 'manifest.json').read_text())
    assert fingerprints(evidence / 'source') == proof['sources']
    assert sha((out / 'bin/publication-checks-probe').read_bytes()) == proof['binary_sha256']
    files = html = 0
    for row in rows:
        actual = json.loads((out / 'inventories' / f"{row['label']}.json").read_text())
        assert actual == json.loads((out / 'inventories' / f"{row['workload']}-expected.json").read_text())
        files += actual['files']
        html += actual['html']
    groups = []
    for workload, scenario in [('xiv', 'same'), ('private', 'same'), ('xiv', 'replace')]:
        selected = [s for s in rows if s['workload'] == workload and s['scenario'] == scenario and s['round'] >= 0]
        names = sorted({k for s in selected for k in s['metrics'] if k.startswith('publish.') or k == 'cli.publish'})
        groups.append({'workload': workload, 'scenario': scenario, 'rounds': len(selected),
                       'metrics': {k: {field: statistics.mean(s['metrics'].get(k, {}).get(field, 0) for s in selected)
                                       for field in ['calls', 'seconds', 'self_seconds']} for k in names}})
    save(evidence / 'summary.json', {'kind': 'after-only-supplement', 'groups': groups})
    save(evidence / 'integrity.json', {'complete_builds': len(rows),
         'file_hashes_verified': files, 'html_outputs_verified': html,
         'all_outputs_identical': True, 'production_and_primary_snapshots_unchanged': True,
         'not_used_for_before_after_speedups': True})


if __name__ == '__main__':
    main()
