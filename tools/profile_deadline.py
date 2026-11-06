#!/usr/bin/env python3
"""Deadline cohort: dirty source freeze, empty-output exec-to-wait4 measurements."""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time

import profile_runtime_cow as common

OUT = Path('/tmp/opencode/notist-deadline-20261105')
PREVIOUS = Path('/tmp/opencode/notist-source-text-20261105')
REPO = Path(__file__).resolve().parents[1]


def prepare():
    OUT.mkdir(exist_ok=False)
    for name in ['sources', 'bin', 'logs']:
        (OUT / name).mkdir()
    files = subprocess.check_output(['git', 'ls-files', '-co', '--exclude-standard', '-z'], cwd=REPO).split(b'\0')
    before = OUT / 'sources/before'
    before.mkdir()
    for raw in sorted(set(files) - {b''}):
        relative = os.fsdecode(raw)
        source = REPO / relative
        if source.is_file():
            target = before / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
    shutil.copytree(PREVIOUS / 'site', OUT / 'site', ignore=shutil.ignore_patterns('dist', '__pycache__'))
    previous = json.loads((PREVIOUS / 'manifest.json').read_text())
    assert common.inventory(OUT / 'site') == previous['inputs']
    old_sources = previous['after_sources']
    assert all(common.digest(before / p) == value[1] for p, value in old_sources.items() if p.endswith(('.rs', '.toml', '.lock')))
    shutil.copy2(PREVIOUS / 'bin/after', OUT / 'bin/before')
    shutil.copy2(PREVIOUS / 'output-inventory.json', OUT / 'output-inventory.json')
    subprocess.run(['git', 'diff', '--binary'], cwd=REPO, stdout=(OUT / 'workspace-initial.patch').open('wb'), check=True)
    manifest = dict(before_sources=common.inventory(before), inputs=previous['inputs'],
                    baseline_binary_sha256=common.digest(OUT / 'bin/before'),
                    baseline_binary_provenance=str(PREVIOUS / 'bin/after'),
                    git_head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(),
                    started_at=subprocess.check_output(['date', '-Ins'], text=True).strip(),
                    cpu_affinity=sorted(os.sched_getaffinity(0)),
                    cache_policy='Filesystem warm allowed; no application-result warmup; output removed before each exec',
                    rss_method='Linux wait4 ru_maxrss KiB, kernel process high-water mark (may include waited-for children; conservative)')
    common.save(OUT / 'manifest.json', manifest)
    print('frozen', len(manifest['before_sources']), 'source files;', len(manifest['inputs']), 'inputs', flush=True)


def run(kinds, rounds, cohort, baseline_rounds=None):
    assert not (OUT / (cohort + '-samples.json')).exists(), 'never overwrite a cohort'
    manifest = json.loads((OUT / 'manifest.json').read_text())
    actual_inputs = common.inventory(OUT / 'site')
    assert {k: v for k, v in actual_inputs.items() if not k.startswith('dist/')} == manifest['inputs']
    expected = json.loads((OUT / 'output-inventory.json').read_text())
    rows = []
    env = dict(os.environ, NO_COLOR='1', TERM='dumb', PYTHONDONTWRITEBYTECODE='1')
    for key in ['LD_PRELOAD', 'RESOURCE_PROFILE', 'RESOURCE_DIAGNOSTIC', 'XIV_PROFILE_REQUESTS', 'EVAL_AB_OBSERVER']:
        assert not env.get(key), key
    for rep in range(rounds):
        if len(kinds) == 3:
            order = [[0, 1, 2], [1, 2, 0], [2, 0, 1], [2, 1, 0], [0, 2, 1], [1, 0, 2]][rep % 6]
            ordered = [kinds[i] for i in order]
        else:
            ordered = kinds if rep % 2 == 0 else kinds[::-1]
        if baseline_rounds is not None and rep >= baseline_rounds:
            ordered = [kind for kind in ordered if kind != 'before']
        for kind in ordered:
            # Reserve 30 seconds for the last process to exit and be verified.
            assert time.time() < 1793851170, 'Beijing 2026-11-05 11:59:30 cutoff for starting work'
            output = OUT / 'site/dist'
            if output.exists():
                shutil.rmtree(output)
            label = f'{cohort}-{rep}-{kind}'
            with (OUT / 'logs' / (label + '.stdout')).open('wb') as stdout, (OUT / 'logs' / (label + '.stderr')).open('wb') as stderr:
                start = time.perf_counter()
                child = subprocess.Popen([str(OUT / 'bin' / kind), 'build'], cwd=OUT / 'site', env=env, stdout=stdout, stderr=stderr)
                _, status, usage = os.wait4(child.pid, 0)
                child.returncode = os.waitstatus_to_exitcode(status)
                wall = time.perf_counter() - start
            row = dict(kind=kind, round=rep, label=label, wall_s=wall, user_s=usage.ru_utime,
                       system_s=usage.ru_stime, maxrss_bytes=usage.ru_maxrss * 1024,
                       major_faults=usage.ru_majflt, minor_faults=usage.ru_minflt,
                       swaps=usage.ru_nswap, exit=child.returncode,
                       load=Path('/proc/loadavg').read_text().strip())
            common.save(OUT / 'logs' / (label + '.json'), row)
            assert child.returncode == 0, row
            actual = common.inventory(OUT / 'site/dist/icexiv-db')
            row['output_verified'] = actual == expected
            assert row['output_verified'], label
            rows.append(row)
            common.save(OUT / (cohort + '-samples.json'), rows)
            print(label, f'{wall:.4f}s', row['maxrss_bytes'], flush=True)
    summary = {kind: dict(wall_median_s=statistics.median(r['wall_s'] for r in rows if r['kind'] == kind),
                          rss_bytes=[r['maxrss_bytes'] for r in rows if r['kind'] == kind]) for kind in kinds}
    common.save(OUT / (cohort + '-summary.json'), summary)
    actual_inputs = common.inventory(OUT / 'site')
    assert {k: v for k, v in actual_inputs.items() if not k.startswith('dist/')} == manifest['inputs']
    print(json.dumps(summary), flush=True)


if __name__ == '__main__':
    p = argparse.ArgumentParser()
    p.add_argument('action', choices=['prepare', 'run'])
    p.add_argument('--kinds', nargs='+', default=['before'])
    p.add_argument('--rounds', type=int, default=6)
    p.add_argument('--cohort', default='baseline')
    p.add_argument('--baseline-rounds', type=int)
    a = p.parse_args()
    prepare() if a.action == 'prepare' else run(a.kinds, a.rounds, a.cohort, a.baseline_rounds)
