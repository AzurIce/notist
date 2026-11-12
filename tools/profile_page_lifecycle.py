#!/usr/bin/env python3
"""Frozen full-site compatibility, wall/CPU/RSS A/B for page ownership changes."""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import time

from profile_support import BUILD_ENV, digest, inventory, save

REPO = Path(__file__).resolve().parents[1]


def prepare(a):
    if a.out.exists():
        raise ValueError('fresh evidence directory required')
    reference = json.loads((a.reference / 'manifest.json').read_text())
    compiler = subprocess.check_output(['rustc', '-Vv'], text=True)
    assert compiler.strip() == reference['rustc'].strip()
    baseline = a.reference / 'bin/after-release'
    assert digest(baseline) == reference['binaries']['after-release']
    assert reference['build_env'] == BUILD_ENV
    a.out.mkdir(parents=True)
    for part in ['bin', 'logs', 'source']:
        (a.out / part).mkdir()
    paths = subprocess.check_output(['git', 'ls-files', '--cached', '--others', '--exclude-standard', '-z'], cwd=REPO)
    for raw in paths.split(b'\0'):
        if not raw:
            continue
        path = Path(os.fsdecode(raw))
        if not (REPO / path).is_file():
            continue
        destination = a.out / 'source' / path
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(REPO / path, destination)
    shutil.copy2(baseline, a.out / 'bin/before')
    shutil.copy2(a.reference / 'output-inventory.json', a.out / 'output-inventory.json')
    shutil.copytree(a.reference / 'site', a.out / 'site',
                    ignore=shutil.ignore_patterns('notist-output', 'target', '__pycache__'))
    assert inventory(a.out / 'site') == reference['inputs']
    save(a.out / 'manifest.json', dict(
        baseline_revision=reference['revisions']['after'],
        reference=str(a.reference), rustc=compiler, build_env=BUILD_ENV,
        base_revision=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(),
        status=subprocess.check_output(['git', 'status', '--short'], cwd=REPO, text=True),
        diff=subprocess.check_output(['git', 'diff', '--binary', 'HEAD'], cwd=REPO, text=True),
        source=inventory(a.out / 'source'), inputs=reference['inputs'],
        binaries={'before': digest(a.out / 'bin/before')},
        system=platform.platform(), affinity=sorted(os.sched_getaffinity(0)),
        started_at=time.strftime('%Y-%m-%dT%H:%M:%S%z')))


def build(a):
    env = dict(os.environ, **BUILD_ENV, CARGO_TARGET_DIR=str(a.out / 'target'))
    for key in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS']:
        env.pop(key, None)
    with (a.out / 'logs/build.log').open('wb') as log:
        subprocess.run(['cargo', 'build', '--release', '--locked', '--offline', '-j2',
                        '-p', 'notist-cli', '--bin', 'notist'],
                       cwd=a.out / 'source', env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(a.out / 'target/release/notist', a.out / 'bin/after')
    m = json.loads((a.out / 'manifest.json').read_text())
    m['binaries']['after'] = digest(a.out / 'bin/after')
    save(a.out / 'manifest.json', m)


def run(a, kind, label, jobs, timings=False):
    usage = a.out / 'logs' / f'{label}.usage.json'
    if usage.exists():
        raise ValueError('never overwrite measurements')
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', NO_COLOR='1', TERM='dumb')
    for key in ['LD_PRELOAD', 'CPUPROFILE', 'GLIBC_TUNABLES', 'MALLOC_ARENA_MAX',
                'MALLOC_TRIM_THRESHOLD_', 'MALLOC_MMAP_THRESHOLD_', 'NOTIST_EVAL_TIMING']:
        env.pop(key, None)
    command = [str(a.out / 'bin' / kind), 'build', '-j', str(jobs)]
    if timings:
        command.append('--timings=json')
    with (a.out / 'logs' / f'{label}.stdout').open('wb') as stdout, (a.out / 'logs' / f'{label}.stderr').open('wb') as stderr:
        start = time.monotonic_ns()
        process = subprocess.Popen(command, cwd=a.out / 'site', env=env, stdout=stdout, stderr=stderr)
        _, status, resource = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
        wall = (time.monotonic_ns() - start) / 1e9
    row = dict(kind=kind, label=label, jobs=jobs, timings=timings, wall_s=wall,
               user_s=resource.ru_utime, system_s=resource.ru_stime,
               cpu_s=resource.ru_utime + resource.ru_stime, max_rss_kib=resource.ru_maxrss,
               minor_faults=resource.ru_minflt, major_faults=resource.ru_majflt,
               returncode=process.returncode, load=Path('/proc/loadavg').read_text().strip())
    save(usage, row)
    assert process.returncode == 0, f'{label}: inspect stderr'
    actual = inventory(a.out / 'site/notist-output/icexiv-db')
    expected = json.loads((a.out / 'output-inventory.json').read_text())
    assert actual == expected, f'{label}: full output mismatch'
    row['files_verified'] = len(actual)
    save(usage, row)
    if timings:
        reports = list((a.out / 'site/target/notist-timings').glob('*.json'))
        shutil.copy2(max(reports, key=lambda p: p.stat().st_mtime_ns), a.out / f'{label}.timings.json')
    print(json.dumps(row), flush=True)
    return row


def summarize(rows):
    return {key: statistics.median(row[key] for row in rows)
            for key in ['wall_s', 'cpu_s', 'user_s', 'system_s', 'max_rss_kib']}


def measure(a):
    rows = [run(a, kind, f'{kind}-warmup', 32) for kind in ['before', 'after']]
    for jobs, rounds in [(32, 5), (16, 3)]:
        for i in range(rounds):
            for kind in (['before', 'after'] if i % 2 == 0 else ['after', 'before']):
                rows.append(run(a, kind, f'{kind}-j{jobs}-{i}', jobs))
                save(a.out / 'samples.json', rows)
    rows.append(run(a, 'after', 'after-serial', 1))
    for i in range(2):
        for kind in ['before', 'after']:
            rows.append(run(a, kind, f'{kind}-timings-{i}', 32, timings=True))
    save(a.out / 'samples.json', rows)
    result = {str(jobs): {kind: summarize([r for r in rows if r['label'].startswith(f'{kind}-j{jobs}-')])
                         for kind in ['before', 'after']} for jobs in [16, 32]}
    save(a.out / 'summary.json', result)
    print(json.dumps(result, indent=2), flush=True)


def verify(a):
    m = json.loads((a.out / 'manifest.json').read_text())
    assert inventory(a.out / 'source') == m['source']
    for kind, expected in m['binaries'].items():
        assert digest(a.out / 'bin' / kind) == expected
    assert {k: v for k, v in inventory(a.out / 'site').items()
            if not k.startswith(('target/', 'notist-output/'))} == m['inputs']
    with (a.out / 'logs/check-site.log').open('wb') as log:
        subprocess.run(['python3', 'scripts/check_site.py', 'notist-output/icexiv-db'],
                       cwd=a.out / 'site', stdout=log, stderr=subprocess.STDOUT, check=True)
    print('All fingerprints and the complete site verified.', flush=True)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=['prepare', 'build', 'measure', 'verify', 'all'])
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--reference', type=Path,
                   default=Path('/tmp/opencode/notist-main-64f0092-profile-20261112'))
    a = p.parse_args()
    a.out = a.out.resolve()
    for name in (['prepare', 'build', 'measure', 'verify'] if a.action == 'all' else [a.action]):
        globals()[name](a)
