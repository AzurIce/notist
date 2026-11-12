#!/usr/bin/env python3
"""Validate resource identity/publication planning against a frozen main release."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import time

import profile_page_lifecycle as site
from profile_support import BUILD_ENV, digest, inventory, save

REPO = Path(__file__).resolve().parents[1]
SCOPED = [
    'src/publication/mod.rs', 'src/publication/index.rs', 'src/publication/plan.rs',
    'src/resource_session.rs', 'src/resources.rs',
    'crates/notist-cli/src/publication.rs', 'tests/publication_plan.rs',
    'docs/designs/resource-publication.not',
]


def prepare(a):
    if a.out.exists():
        raise ValueError('fresh evidence directory required')
    reference = json.loads((a.reference / 'manifest.json').read_text())
    compiler = subprocess.check_output(['rustc', '-Vv'], text=True)
    assert compiler.strip() == reference['rustc'].strip()
    assert reference['build_env'] == BUILD_ENV
    assert digest(a.reference / 'bin/release') == reference['binaries']['release']
    a.out.mkdir(parents=True)
    for part in ['source', 'bin', 'logs', 'reproducer']:
        (a.out / part).mkdir()
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    archive = subprocess.check_output(['git', 'archive', head], cwd=REPO)
    with tarfile.open(fileobj=io.BytesIO(archive)) as tree:
        tree.extractall(a.out / 'source', filter='data')
    # Overlay only the task's explicit files. Unrelated uncommitted work does
    # not enter the frozen build, even if it happens to be tracked documentation.
    for name in SCOPED:
        target = a.out / 'source' / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(REPO / name, target)
    shutil.copy2(a.reference / 'bin/release', a.out / 'bin/before')
    shutil.copy2(a.reference / 'output-inventory.json', a.out / 'output-inventory.json')
    shutil.copytree(a.reference / 'site', a.out / 'site',
                    ignore=shutil.ignore_patterns('notist-output', 'target', '__pycache__'))
    assert inventory(a.out / 'site') == reference['inputs']
    save(a.out / 'manifest.json', dict(base_revision=head, baseline_revision=reference['revision'],
        reference=str(a.reference), rustc=compiler, build_env=BUILD_ENV,
        scoped=SCOPED, source=inventory(a.out / 'source'), inputs=reference['inputs'],
        binaries={'before': digest(a.out / 'bin/before')},
        diff=subprocess.check_output(['git', 'diff', '--binary', 'HEAD', '--', *SCOPED], cwd=REPO, text=True),
        started_at=time.strftime('%Y-%m-%dT%H:%M:%S%z')))


def build(a):
    def one(label):
        env = dict(os.environ, **BUILD_ENV, CARGO_TARGET_DIR=str(a.out / f'target-{label}'))
        for key in ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS']:
            env.pop(key, None)
        if label == 'after-frames':
            env['RUSTFLAGS'] = '-C force-frame-pointers=yes'
        with (a.out / 'logs' / f'build-{label}.log').open('wb') as log:
            subprocess.run(['cargo', 'build', '--release', '--locked', '--offline', '-j2',
                            '-p', 'notist-cli', '--bin', 'notist'], cwd=a.out / 'source',
                           env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(a.out / f'target-{label}/release/notist', a.out / 'bin' / label)
        return label, digest(a.out / 'bin' / label)
    with ThreadPoolExecutor(max_workers=2) as pool:
        hashes = dict(pool.map(one, ['after', 'after-frames']))
    m = json.loads((a.out / 'manifest.json').read_text())
    m['binaries'].update(hashes)
    save(a.out / 'manifest.json', m)


def perf(a, i):
    label = f'after-perf-{i}'
    assert not (a.out / 'logs' / f'{label}.usage.json').exists()
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', NO_COLOR='1', TERM='dumb')
    for key in ['LD_PRELOAD', 'CPUPROFILE', 'GLIBC_TUNABLES', 'MALLOC_ARENA_MAX',
                'MALLOC_TRIM_THRESHOLD_', 'MALLOC_MMAP_THRESHOLD_', 'NOTIST_EVAL_TIMING']:
        env.pop(key, None)
    with (a.out / 'logs' / f'{label}.stdout').open('wb') as stdout, (a.out / 'logs' / f'{label}.stderr').open('wb') as stderr:
        start = time.monotonic_ns()
        process = subprocess.Popen(['perf', 'record', '-e', 'cycles:u', '-F', '499', '--call-graph', 'fp',
            '-o', str(a.out / f'{label}.data'), '--', str(a.out / 'bin/after-frames'), 'build', '-j32'],
            cwd=a.out / 'site', env=env, stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(process.pid, 0)
        process.returncode = os.waitstatus_to_exitcode(status)
        elapsed = (time.monotonic_ns() - start) / 1e9
    assert process.returncode == 0, f'{label}: inspect stderr'
    row = dict(label=label, kind='after', mode='perf', jobs=32, wall_s=elapsed,
        user_s=usage.ru_utime, system_s=usage.ru_stime, cpu_s=usage.ru_utime + usage.ru_stime,
        max_rss_kib=usage.ru_maxrss, files_verified=site_output_check(a))
    save(a.out / 'logs' / f'{label}.usage.json', row)
    for name, flags in [('flat', ['--no-children', '--no-inline']), ('cumulative', ['--children'])]:
        with (a.out / f'{label}-{name}.txt').open('wb') as log:
            subprocess.run(['perf', 'report', '--stdio', '-g', 'none', '--sort', 'dso,symbol',
                '--show-total-period', '--percent-limit', '0', '--field-separator', '\t', *flags,
                '-i', str(a.out / f'{label}.data')], stdout=log, stderr=subprocess.STDOUT, check=True)
    print(json.dumps(row), flush=True)
    return row


def site_output_check(a):
    actual = inventory(a.out / 'site/notist-output/icexiv-db')
    assert actual == json.loads((a.out / 'output-inventory.json').read_text())
    return len(actual)


def measure(a):
    rows = [site.run(a, kind, f'{kind}-warmup', 32) for kind in ['before', 'after']]
    for jobs, rounds in [(32, 5), (8, 3), (16, 3)]:
        for i in range(rounds):
            for kind in (['before', 'after'] if i % 2 == 0 else ['after', 'before']):
                rows.append(site.run(a, kind, f'{kind}-j{jobs}-{i}', jobs))
                save(a.out / 'samples.json', rows)
    rows.append(site.run(a, 'after', 'after-serial', 1))
    for i in range(2):
        for kind in ['before', 'after']:
            rows.append(site.run(a, kind, f'{kind}-timings-{i}', 32, timings=True))
            save(a.out / 'samples.json', rows)
    for i in range(3):
        rows.append(perf(a, i))
        save(a.out / 'samples.json', rows)
    summary = {str(j): {kind: site.summarize([r for r in rows if r['label'].startswith(f'{kind}-j{j}-')])
                       for kind in ['before', 'after']} for j in [8, 16, 32]}
    save(a.out / 'summary.json', summary)
    print(json.dumps(summary, indent=2), flush=True)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=['prepare', 'build', 'measure', 'verify', 'all'])
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--reference', type=Path,
                   default=Path('/tmp/opencode/notist-main-5187881-profile-20261112'))
    a = p.parse_args()
    a.out = a.out.resolve()
    for name in (['prepare', 'build', 'measure', 'verify'] if a.action == 'all' else [a.action]):
        (site.verify if name == 'verify' else globals()[name])(a)
