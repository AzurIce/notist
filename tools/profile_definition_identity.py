#!/usr/bin/env python3
"""Freeze DefId candidates and compare against the accepted SourceId build.

Consumer inputs are reused read-only; only their frozen dist is republished.
Clean release timings and separately instrumented allocation ledgers never mix.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess

from profile_icexiv_refresh import freeze
from profile_source_identity import BUILD_ENV, digest, inventory, save
from profile_source_cost import instrument, run, summarize

REPO = Path(__file__).resolve().parents[1]


def prepare(a):
    if a.out.exists() or a.out.is_relative_to(REPO):
        raise ValueError('use a fresh external evidence directory')
    prior = json.loads((a.reference / 'manifest.json').read_text())
    for directory in ['bin', 'logs']:
        (a.out / directory).mkdir(parents=True)
    freeze(a.out / 'sources/after')
    shutil.copytree(a.reference / 'source', a.out / 'sources/before')
    shutil.copy2(a.reference / 'bin/after', a.out / 'bin/before')
    assert digest(a.out / 'bin/before') == prior['binaries']['after']
    (a.out / 'site').symlink_to((a.reference / 'site').resolve(), target_is_directory=True)
    shutil.copy2(a.reference / 'output-inventory.json', a.out / 'output-inventory.json')
    inputs = {k: v for k, v in inventory(a.out / 'site').items() if not k.startswith('dist/')}
    original = json.loads((Path(prior['reference']) / 'manifest.json').read_text())['inputs']
    assert inputs == original
    save(a.out / 'manifest.json', dict(
        reference=str(a.reference), build_env=BUILD_ENV, inputs=inputs,
        sources={kind: inventory(a.out / 'sources' / kind) for kind in ['before', 'after']},
        binaries={'before': digest(a.out / 'bin/before')},
        revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(),
    ))


def verify(a):
    m = json.loads((a.out / 'manifest.json').read_text())
    for kind, expected in m['sources'].items():
        assert inventory(a.out / 'sources' / kind) == expected, kind
    for kind, expected in m['binaries'].items():
        assert digest(a.out / 'bin' / kind) == expected, kind
    inputs = {k: v for k, v in inventory(a.out / 'site').items() if not k.startswith('dist/')}
    assert inputs == m['inputs']
    if (a.out / 'clean-samples.json').exists():
        assert inventory(a.out / 'site/dist/icexiv-db') == json.loads((a.out / 'output-inventory.json').read_text())
    return m


def build_kind(a, kind):
    m = verify(a)
    if kind in m['binaries']:
        raise ValueError('never replace a recorded executable: ' + kind)
    # Copied source mtimes can be older than a previous build in a shared target.
    # Binding-identity acceptance isolates every frozen revision (including probes).
    target = a.target / kind if getattr(a, 'isolated_targets', False) else a.target
    env = dict(os.environ, **BUILD_ENV, CARGO_TARGET_DIR=str(target))
    with (a.out / 'logs' / f'build-{kind}.log').open('wb') as log:
        subprocess.run(['cargo', 'build', '--release', '--locked', '--offline', '-p', 'notist-cli', '--bin', 'notist'],
                       cwd=a.out / 'sources' / kind, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(target / 'release/notist', a.out / 'bin' / kind)
    m['binaries'][kind] = digest(a.out / 'bin' / kind)
    save(a.out / 'manifest.json', m)


def build(a):
    build_kind(a, 'after')


def measure(a):
    verify(a)
    if (a.out / 'clean-samples.json').exists():
        raise ValueError('do not overwrite measurements')
    for kind in ['before', 'after']:
        run(a, kind, f'warmup-{kind}')
    rows = []
    for pair in range(a.pairs):
        for kind in (['before', 'after'] if pair % 2 == 0 else ['after', 'before']):
            rows.append(run(a, kind, f'clean-{pair}-{kind}'))
            save(a.out / 'clean-samples.json', rows)
    summary = summarize(rows)
    ratios = [next(r['wall_s'] for r in rows if r['label'] == f'clean-{i}-after') /
              next(r['wall_s'] for r in rows if r['label'] == f'clean-{i}-before') for i in range(a.pairs)]
    summary['paired_wall'] = dict(ratios=ratios, median_percent=100 * (statistics.median(ratios) - 1))
    save(a.out / 'clean-summary.json', summary)
    verify(a)


def ledgers(a):
    for parent in ['before', 'after']:
        kind = parent + '-ledger'
        m = verify(a)
        if kind in m['sources']:
            raise ValueError('do not overwrite diagnostics')
        shutil.copytree(a.out / 'sources' / parent, a.out / 'sources' / kind)
        instrument(a.out / 'sources' / kind, scoped=False)
        m['sources'][kind] = inventory(a.out / 'sources' / kind)
        save(a.out / 'manifest.json', m)
        build_kind(a, kind)
    rows = []
    for kind in ['before-ledger', 'after-ledger']:
        row = run(a, kind, kind, rss=True)
        text = (a.out / 'logs' / f'{kind}.stderr').read_text()
        for prefix, field in [('[source-cost] ', 'scopes'), ('[source-memory] ', 'memory')]:
            row[field] = [json.loads(line[len(prefix):]) for line in text.splitlines() if line.startswith(prefix)]
        assert len(row['scopes']) == 16 and row['memory']
        rows.append(row)
    save(a.out / 'ledger.json', rows)
    verify(a)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=['prepare', 'build', 'measure', 'ledgers', 'verify'])
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--reference', type=Path, default=Path('/tmp/opencode/notist-source-walk-fix-20261108'))
    p.add_argument('--target', type=Path, default=Path('/tmp/opencode/notist-source-identity-complete-20261107/target'))
    p.add_argument('--pairs', type=int, default=8)
    a = p.parse_args()
    a.out, a.reference, a.target = a.out.resolve(), a.reference.resolve(), a.target.resolve()
    if not 1 <= a.pairs <= 20:
        p.error('--pairs must be 1..20')
    globals()[a.action](a)
