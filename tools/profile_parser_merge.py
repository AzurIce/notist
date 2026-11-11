#!/usr/bin/env python3
"""Compare two committed revisions on one frozen icexiv-db input snapshot.

Clean release runs are interleaved; frame-pointer CPU samples and --timings
are separate diagnostics. Never builds or publishes in the consumer checkout.
"""
import argparse
from collections import defaultdict
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tarfile
import time

from profile_support import BUILD_ENV, digest, inventory, save

REPO = Path(__file__).resolve().parents[1]
INPUTS = ['Notist.toml', 'content', 'data', 'inputs', 'modules', 'scripts',
          'sources', 'static', 'templates']
PROFILER = Path('/nix/store/ca8m3rhpgmd0q7k47cb9gb3a7622jdpc-gperftools-2.18.1/lib/libprofiler.so.0.5.19')
PPROF = Path('/nix/store/qrns8m17akfs90vcgj672q4bzx7gj1qd-gperftools-2.15/bin/pprof')


def git(*args):
    return subprocess.check_output(['git', *args], cwd=REPO, text=True).strip()


def prepare(a):
    if a.out.exists():
        raise ValueError('fresh evidence directory required')
    a.out.mkdir(parents=True)
    for directory in ['sources', 'bin', 'logs', 'reproducer']:
        (a.out / directory).mkdir()
    revisions = {k: git('rev-parse', v) for k, v in [('before', a.before), ('after', a.after)]}
    for kind, revision in revisions.items():
        archive = a.out / f'{kind}.tar'
        subprocess.run(['git', 'archive', '-o', str(archive), revision], cwd=REPO, check=True)
        root = a.out / 'sources' / kind
        root.mkdir()
        with tarfile.open(archive) as stream:
            stream.extractall(root, filter='data')
        archive.unlink()
    for name in INPUTS:
        src, dst = a.consumer / name, a.out / 'site' / name
        dst.parent.mkdir(parents=True, exist_ok=True)
        if src.is_dir():
            shutil.copytree(src, dst, ignore=shutil.ignore_patterns('__pycache__'),
                            ignore_dangling_symlinks=True)
        else:
            shutil.copy2(src, dst)
    for name in ['profile_parser_merge.py', 'profile_support.py', 'hotspot_cpu_guard.c']:
        shutil.copy2(REPO / 'tools' / name, a.out / 'reproducer' / name)
    save(a.out / 'manifest.json', {
        'revisions': revisions, 'consumer_revision': subprocess.check_output(
            ['git', 'rev-parse', 'HEAD'], cwd=a.consumer, text=True).strip(),
        'consumer_status': subprocess.check_output(['git', 'status', '--short'], cwd=a.consumer, text=True),
        'inputs': inventory(a.out / 'site'),
        'sources': {k: inventory(a.out / 'sources' / k) for k in revisions},
        'rustc': subprocess.check_output(['rustc', '-Vv'], text=True),
        'system': platform.platform(), 'affinity': sorted(os.sched_getaffinity(0)),
        'build_env': BUILD_ENV, 'binaries': {},
        'note': 'Warm filesystem, fresh process, full build incl. Python extension and publication; CPU sampling parent only.',
    })


def build(a):
    m = json.loads((a.out / 'manifest.json').read_text())
    for kind in ['before', 'after']:
        assert inventory(a.out / 'sources' / kind) == m['sources'][kind]
        for variant in ['release', 'frames']:
            label = f'{kind}-{variant}'
            if label in m['binaries']:
                continue
            target = a.out / 'targets' / label
            env = dict(os.environ, **BUILD_ENV, CARGO_TARGET_DIR=str(target))
            if variant == 'frames':
                env['RUSTFLAGS'] = '-C force-frame-pointers=yes'
            with (a.out / 'logs' / f'build-{label}.log').open('wb') as log:
                subprocess.run(['cargo', 'build', '--release', '--locked', '--offline', '-j', '2',
                                '-p', 'notist-cli', '--bin', 'notist'],
                               cwd=a.out / 'sources' / kind, env=env,
                               stdout=log, stderr=subprocess.STDOUT, check=True)
            shutil.copy2(target / 'release/notist', a.out / 'bin' / label)
            m['binaries'][label] = digest(a.out / 'bin' / label)
            save(a.out / 'manifest.json', m)
            print('built', label, flush=True)


def run(a, kind, label, variant='release', diagnostic=None):
    binary = a.out / 'bin' / f'{kind}-{variant}'
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', NO_COLOR='1', TERM='dumb')
    for key in ['LD_PRELOAD', 'CPUPROFILE', 'RUSTFLAGS']:
        env.pop(key, None)
    command = [str(binary), 'build']
    if diagnostic == 'timings':
        command += ['--timings=json']
    if diagnostic == 'cpu':
        env.update(LD_PRELOAD=str(a.out / 'bin/cpu-guard.so'), HOTSPOT_CPU_TARGET=str(binary),
                   HOTSPOT_CPU_OUTPUT=str(a.out / f'{label}.prof'), CPUPROFILE_FREQUENCY='499')
    if diagnostic == 'perf':
        command = ['perf', 'record', '-e', 'cycles:u', '-F', '499',
                   '--call-graph', 'fp', '-o', str(a.out / f'{label}.data'), '--', *command]
    if diagnostic == 'stat':
        command = ['perf', 'stat', '--json-output', '-o', str(a.out / f'{label}-stat.json'),
                   '-e', 'cycles:u,instructions:u,cache-references:u,cache-misses:u,branches:u,branch-misses:u',
                   '--', *command]
    # wait4 reports the command's own high-water RSS and CPU (incl. children),
    # unlike cumulative RUSAGE_CHILDREN.ru_maxrss in a long-lived driver.
    with (a.out / 'logs' / f'{label}.stdout').open('wb') as stdout, (a.out / 'logs' / f'{label}.stderr').open('wb') as stderr:
        start = time.perf_counter()
        proc = subprocess.Popen(command, cwd=a.out / 'site', env=env, stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(proc.pid, 0)
        proc.returncode = os.waitstatus_to_exitcode(status)
        elapsed = time.perf_counter() - start
        if proc.returncode:
            raise RuntimeError(f'{label} failed: see logs')
    actual = inventory(a.out / 'site/notist-output/icexiv-db')
    expected = a.out / 'output-inventory.json'
    if expected.exists():
        assert actual == json.loads(expected.read_text()), f'output mismatch: {label}'
    else:
        assert any(p.endswith('.html') for p in actual), 'no HTML generated'
        save(expected, actual)
    if diagnostic == 'timings':
        reports = sorted((a.out / 'site/target/notist-timings').glob('*.json'), key=lambda p: p.stat().st_mtime_ns)
        shutil.copy2(reports[-1], a.out / f'{label}.json')
    row = dict(kind=kind, label=label, wall_s=elapsed, user_s=usage.ru_utime,
               system_s=usage.ru_stime, max_rss_kib=usage.ru_maxrss, files=len(actual))
    print(json.dumps(row), flush=True)
    return row


def measure(a):
    if (a.out / 'samples.json').exists():
        raise ValueError('never overwrite measurements')
    rows = []
    for kind in ['before', 'after']:
        rows.append(run(a, kind, f'{kind}-warmup'))
    for i in range(a.rounds):
        for kind in (['before', 'after'] if i % 2 == 0 else ['after', 'before']):
            rows.append(run(a, kind, f'{kind}-{i}'))
            save(a.out / 'samples.json', rows)
    result = {}
    for kind in ['before', 'after']:
        clean = [r for r in rows if r['kind'] == kind and 'warmup' not in r['label']]
        result[kind] = {key: statistics.median(r[key] for r in clean)
                        for key in ['wall_s', 'user_s', 'system_s', 'max_rss_kib']}
    result['wall_speedup'] = result['before']['wall_s'] / result['after']['wall_s']
    save(a.out / 'summary.json', result)


def profile(a):
    subprocess.run(['cc', '-shared', '-O2', '-fPIC', str(a.out / 'reproducer/hotspot_cpu_guard.c'),
                    str(PROFILER), '-Wl,-rpath,' + str(PROFILER.parent), '-o',
                    str(a.out / 'bin/cpu-guard.so')], check=True)
    rows = []
    for kind in ['before', 'after']:
        rows.append(run(a, kind, f'{kind}-timings', diagnostic='timings'))
        for i in range(2):
            label = f'{kind}-cpu-{i}'
            if (a.out / f'{label}.prof').exists():
                raise ValueError('never overwrite CPU profile')
            rows.append(run(a, kind, label, 'frames', 'cpu'))
            for name, flags in [('self', []), ('cumulative', ['--cum']), ('collapsed', ['--collapsed'])]:
                with (a.out / f'{label}-{name}.txt').open('wb') as log:
                    subprocess.run([str(PPROF), '--text' if name != 'collapsed' else '--collapsed',
                                    '--nodefraction=0', *flags,
                                    str(a.out / 'bin' / f'{kind}-frames'), str(a.out / f'{label}.prof')],
                                   stdout=log, stderr=subprocess.STDOUT, check=True)
    save(a.out / 'diagnostics.json', rows)


def perf(a):
    """Hardware-event diagnostics, separate from clean release measurements."""
    rows = []
    for kind in ['before', 'after']:
        rows.append(run(a, kind, f'{kind}-timings', diagnostic='timings'))
        for i in range(2):
            label = f'{kind}-perf-{i}'
            if (a.out / f'{label}.data').exists():
                raise ValueError('never overwrite perf evidence')
            rows.append(run(a, kind, label, 'frames', 'perf'))
            for name, flags in [('self', ['--no-children']), ('cumulative', ['--children'])]:
                with (a.out / f'{label}-{name}.txt').open('wb') as log:
                    subprocess.run(['perf', 'report', '--stdio', '--call-graph', 'none',
                                    '--percent-limit', '0.2', '--sort', 'dso,symbol', *flags,
                                    '-i', str(a.out / f'{label}.data')],
                                   stdout=log, stderr=subprocess.STDOUT, check=True)
            with (a.out / f'{label}-stacks.txt').open('wb') as log:
                subprocess.run(['perf', 'script', '-i', str(a.out / f'{label}.data')],
                               stdout=log, stderr=subprocess.STDOUT, check=True)
            with (a.out / f'{label}-flat.txt').open('wb') as log:
                subprocess.run(['perf', 'report', '--stdio', '--no-children', '--no-inline',
                                '-g', 'none', '--sort', 'dso,symbol', '--percent-limit', '0.2',
                                '-i', str(a.out / f'{label}.data')],
                               stdout=log, stderr=subprocess.STDOUT, check=True)
            rows.append(run(a, kind, f'{kind}-stat-{i}', diagnostic='stat'))
    save(a.out / 'perf-diagnostics.json', rows)
    m = json.loads((a.out / 'manifest.json').read_text())
    m['perf'] = {'version': subprocess.check_output(['perf', 'version'], text=True).strip(),
                 'paranoid': Path('/proc/sys/kernel/perf_event_paranoid').read_text().strip(),
                 'event': 'cycles:u', 'frequency': 499, 'call_graph': 'fp',
                 'scope': 'Rust parent, worker threads and inherited children; user mode only'}
    save(a.out / 'manifest.json', m)
    extra = []
    for i in [1, 2]:
        for kind in (['before', 'after'] if i == 1 else ['after', 'before']):
            extra.append(run(a, kind, f'{kind}-timings-{i}', diagnostic='timings'))
            save(a.out / 'extra-timings-samples.json', extra)


def timing_summary(report):
    groups = defaultdict(list)
    for span in report['spans']:
        groups[span['stage']].append((span['start_ns'], span['end_ns']))
    result = {}
    for stage, intervals in groups.items():
        union = 0
        right = 0
        for start, end in sorted(intervals):
            union += max(0, end - max(start, right))
            right = max(right, end)
        result[stage] = {'calls': len(intervals),
                         'inclusive_s': sum(end - start for start, end in intervals) / 1e9,
                         'interval_union_s': union / 1e9}
    return result


def report(a):
    samples = {kind: [timing_summary(json.loads(path.read_text()))
                      for path in sorted(a.out.glob(f'{kind}-timings*.json'))]
               for kind in ['before', 'after']}
    stages = {
        kind: {stage: {key: statistics.median(sample[stage][key] for sample in runs)
                       for key in runs[0][stage]}
               for stage in runs[0]}
        for kind, runs in samples.items()
    }
    save(a.out / 'stage-samples.json', samples)
    save(a.out / 'stage-summary.json', stages)
    print(json.dumps(json.loads((a.out / 'summary.json').read_text()), indent=2))
    print(json.dumps(stages, indent=2))


def verify(a):
    m = json.loads((a.out / 'manifest.json').read_text())
    for kind, expected in m['sources'].items():
        assert inventory(a.out / 'sources' / kind) == expected
    for label, expected in m['binaries'].items():
        assert digest(a.out / 'bin' / label) == expected
    inputs = {k: v for k, v in inventory(a.out / 'site').items()
              if not k.startswith(('notist-output/', 'target/'))}
    assert inputs == m['inputs'], 'frozen inputs changed'
    assert inventory(a.out / 'site/notist-output/icexiv-db') == json.loads((a.out / 'output-inventory.json').read_text())
    with (a.out / 'logs/check-site.log').open('wb') as log:
        subprocess.run(['python3', 'scripts/check_site.py', 'notist-output/icexiv-db'], cwd=a.out / 'site',
                       stdout=log, stderr=subprocess.STDOUT, check=True)
    print('Verified sources, binaries, frozen inputs, byte-identical full outputs and check_site.', flush=True)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=['prepare', 'build', 'measure', 'profile', 'perf', 'report', 'verify', 'all'])
    p.add_argument('--out', type=Path, required=True)
    p.add_argument('--consumer', type=Path, default=Path('/home/azurice/Files/_ffxiv/icexiv-db'))
    p.add_argument('--before', default='119c17d')
    p.add_argument('--after', default='HEAD')
    p.add_argument('--rounds', type=int, default=5)
    a = p.parse_args()
    a.out = a.out.resolve()
    for action in (['prepare', 'build', 'measure', 'perf', 'verify', 'report'] if a.action == 'all' else [a.action]):
        globals()[action](a)
