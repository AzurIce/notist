#!/usr/bin/env python3
"""Frozen full-site path-reuse A/B: clean timings and separate allocation probes.

The reference must be a verified profile_hotspots output for the current HEAD.
Inputs are copied independently; every run checks the complete output inventory.
"""
import argparse
import json
import os
from pathlib import Path
import random
import shutil
import statistics
import subprocess

from analyze_source_cost import alloc_stacks
from profile_icexiv_refresh import freeze
from profile_source_cost import run, summarize
from profile_support import BUILD_ENV, digest, inventory, save

REPO = Path(__file__).resolve().parents[1]


def prepare(a):
    if a.out.exists() or a.out.is_relative_to(REPO):
        raise ValueError('use a fresh external evidence directory')
    prior = json.loads((a.reference / 'manifest.json').read_text())
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    if revision != prior['revision']:
        raise ValueError('reference must match candidate base revision')
    for name in ['bin', 'logs', 'sources', 'reproducer']:
        (a.out / name).mkdir(parents=True)
    freeze(a.out / 'sources/after')
    shutil.copytree(a.reference / 'site', a.out / 'site', symlinks=True,
                    ignore=shutil.ignore_patterns('dist', '__pycache__'))
    assert inventory(a.out / 'site') == prior['inputs']
    shutil.copy2(a.reference / 'output-inventory.json', a.out / 'output-inventory.json')
    binaries = {}
    for source, target in [('normal', 'before'), ('frames', 'frames-before')]:
        binary = a.reference / 'bin' / source
        assert digest(binary) == prior['binaries'][source]
        shutil.copy2(binary, a.out / 'bin' / target)
        binaries[target] = digest(binary)
    shutil.copy2(a.reference / 'bin/alloc-stacks.so', a.out / 'bin/alloc-stacks.so')
    for tool in (REPO / 'tools').iterdir():
        if tool.is_file():
            shutil.copy2(tool, a.out / 'reproducer' / tool.name)
    (a.out / 'candidate.patch').write_bytes(subprocess.check_output(
        ['git', 'diff', '--binary', 'HEAD'], cwd=REPO))
    save(a.out / 'manifest.json', dict(
        revision=revision, reference=str(a.reference), inputs=prior['inputs'],
        sources={'after': inventory(a.out / 'sources/after')}, binaries=binaries,
        build_env=BUILD_ENV, allocation_probe=digest(a.out / 'bin/alloc-stacks.so')))


def build(a):
    m = json.loads((a.out / 'manifest.json').read_text())
    source = a.out / 'sources/after'
    assert inventory(source) == m['sources']['after']
    for kind in ['after', 'frames-after']:
        if kind in m['binaries']:
            raise ValueError('never replace recorded executable: ' + kind)
        target = a.out / 'targets' / kind
        env = dict(os.environ, **BUILD_ENV, CARGO_TARGET_DIR=str(target))
        if kind.startswith('frames-'):
            env['RUSTFLAGS'] = '-C force-frame-pointers=yes'
        with (a.out / 'logs' / f'build-{kind}.log').open('wb') as log:
            subprocess.run(['cargo', 'build', '--release', '--locked', '--offline',
                            '-p', 'notist-cli', '--bin', 'notist'], cwd=source, env=env,
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(target / 'release/notist', a.out / 'bin' / kind)
        m['binaries'][kind] = digest(a.out / 'bin' / kind)
        save(a.out / 'manifest.json', m)
        print('built', kind, flush=True)


def measure(a):
    if (a.out / 'clean-samples.json').exists():
        raise ValueError('never overwrite measurements')
    for kind in ['before', 'after']:
        run(a, kind, 'warmup-' + kind)
    rows = []
    for i in range(a.pairs):
        for kind in (['before', 'after'] if i % 2 == 0 else ['after', 'before']):
            rows.append(run(a, kind, f'clean-{i}-{kind}'))
            save(a.out / 'clean-samples.json', rows)
    summary = summarize(rows)
    ratios = [next(r['wall_s'] for r in rows if r['label'] == f'clean-{i}-after') /
              next(r['wall_s'] for r in rows if r['label'] == f'clean-{i}-before')
              for i in range(a.pairs)]
    rng = random.Random(711)
    boot = sorted(statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(20000))
    summary['paired_wall'] = dict(ratios=ratios,
        median_percent=100 * (statistics.median(ratios) - 1),
        bootstrap_95_percent=[100 * (boot[500] - 1), 100 * (boot[19499] - 1)],
        warning='Local interleaved pairs, not independent machine replications.')
    save(a.out / 'clean-summary.json', summary)


def allocations(a):
    rows = []
    for kind in ['frames-before', 'frames-after']:
        label = 'alloc-' + kind
        rows.append(run(a, kind, label, extra={
            'LD_PRELOAD': str(a.out / 'bin/alloc-stacks.so'),
            'SOURCE_ALLOC_TARGET': str(a.out / 'bin' / kind),
            'SOURCE_ALLOC_STACKS': str(a.out / (label + '.bin')),
        }))
        save(a.out / 'alloc-samples.json', rows)
    alloc_stacks(a.out)


def verify(a):
    m = json.loads((a.out / 'manifest.json').read_text())
    assert inventory(a.out / 'sources/after') == m['sources']['after']
    for kind, expected in m['binaries'].items():
        assert digest(a.out / 'bin' / kind) == expected
    assert digest(a.out / 'bin/alloc-stacks.so') == m['allocation_probe']
    assert {k: v for k, v in inventory(a.out / 'site').items()
            if not k.startswith('dist/')} == m['inputs']
    assert inventory(a.out / 'site/dist/icexiv-db') == json.loads(
        (a.out / 'output-inventory.json').read_text())
    print('frozen candidate, executables, inputs and full outputs verified', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'build', 'measure', 'allocations', 'verify'])
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--reference', required=True, type=Path)
    parser.add_argument('--pairs', type=int, default=7)
    args = parser.parse_args()
    if args.pairs < 2:
        parser.error('--pairs must be at least 2')
    args.out = args.out.resolve()
    args.reference = args.reference.resolve()
    globals()[args.action](args)
