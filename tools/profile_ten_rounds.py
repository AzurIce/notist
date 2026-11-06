#!/usr/bin/env python3
"""Frozen ten-experiment campaign. No deadline and no application-cache warmup."""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time

import profile_runtime_cow as common

OUT = Path('/tmp/opencode/notist-ten-rounds-20261105')
OLD = Path('/tmp/opencode/notist-deadline-20261105')
REPO = Path(__file__).resolve().parents[1]


def freeze(destination):
    destination.mkdir()
    names = subprocess.check_output(['git', 'ls-files', '-co', '--exclude-standard', '-z'], cwd=REPO).split(b'\0')
    for raw in sorted(set(names) - {b''}):
        source = REPO / os.fsdecode(raw)
        if source.is_file():
            target = destination / source.relative_to(REPO)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)
    return common.inventory(destination)


def prepare():
    OUT.mkdir(exist_ok=False)
    for name in ['sources', 'bin', 'logs', 'reproducer']:
        (OUT / name).mkdir()
    sources = freeze(OUT / 'sources/r00')
    previous = json.loads((OLD / 'verified-source-inventory.json').read_text())
    assert all(sources[p] == value for p, value in previous.items() if p.endswith(('.rs', '.toml', '.lock')))
    shutil.copytree(OLD / 'site', OUT / 'site', ignore=shutil.ignore_patterns('dist', '__pycache__'))
    inputs = common.inventory(OUT / 'site')
    assert inputs == json.loads((OLD / 'manifest.json').read_text())['inputs']
    shutil.copy2(OLD / 'output-inventory.json', OUT / 'output-inventory.json')
    shutil.copy2(OLD / 'bin/verified', OUT / 'bin/r00')
    for name in ['profile_ten_rounds.py', 'profile_runtime_cow.py', 'profile_serve_edits.py']:
        shutil.copy2(REPO / 'tools' / name, OUT / 'reproducer' / name)
    with (OUT / 'initial-workspace.patch').open('wb') as log:
        subprocess.run(['git', 'diff', '--binary'], cwd=REPO, stdout=log, check=True)
    manifest = dict(max_experiments=10, experiments=[], inputs=inputs, sources={'r00': sources},
                    binaries={'r00': common.digest(OUT / 'bin/r00')},
                    git_head=subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip(),
                    rustc=subprocess.check_output(['rustc', '-Vv'], text=True),
                    cpu_affinity=sorted(os.sched_getaffinity(0)), started_at=subprocess.check_output(['date', '-Ins'], text=True).strip(),
                    baseline_provenance=str(OLD / 'bin/verified'),
                    cache_policy='Full frozen input inventory reads warm filesystem only; each build new process, empty output, no result cache warmup',
                    rss_method='wait4 ru_maxrss*1024 kernel high-water mark; conservative maximum including waited-for children, not sparse sampling')
    common.save(OUT / 'manifest.json', manifest)
    print('frozen', len(sources), 'source files and', len(inputs), 'input files', flush=True)


def guard():
    manifest = json.loads((OUT / 'manifest.json').read_text())
    actual = common.inventory(OUT / 'site')
    assert {k: v for k, v in actual.items() if not k.startswith('dist/')} == manifest['inputs']
    return manifest


def begin(args):
    manifest = guard()
    assert all(e['state'] in ['accepted', 'rejected'] for e in manifest['experiments']), 'Finish the current experiment first'
    number = len(manifest['experiments']) + 1
    assert number <= manifest['max_experiments'], 'Ten-experiment budget exhausted'
    assert args.name == f'r{number:02d}'
    assert args.control in manifest['binaries']
    assert args.hypothesis
    manifest['experiments'].append(dict(number=number, name=args.name, control=args.control,
                                         hypothesis=args.hypothesis, state='started'))
    common.save(OUT / 'manifest.json', manifest)
    print('started experiment', number, args.hypothesis, flush=True)


def resolve(args):
    manifest = guard()
    experiment = manifest['experiments'][-1]
    assert experiment['name'] == args.name and experiment['state'] == 'started'
    assert args.reason
    assert args.outcome in ['accepted', 'rejected']
    if args.outcome == 'accepted':
        summary = json.loads((OUT / (args.name + '-ab-summary.json')).read_text())
        before, after = summary[experiment['control']], summary[args.name]
        assert before['n'] >= 6 and after['n'] >= 6
        assert after['wall_median_s'] <= before['wall_median_s']
        assert after['maxrss_bytes'] <= before['maxrss_bytes']
        tests = json.loads((OUT / (args.name + '-tests.json')).read_text())
        assert {t['mode'] for t in tests} == {'default', 'all', 'no-default'}
        assert all(t['exit'] == 0 and t['passed'] > 0 and t['warnings'] == 0 for t in tests)
        semantic = json.loads((OUT / (args.name + '-full-semantic.json')).read_text())
        assert semantic['identical'] and semantic['frames'] == 2*51_739+1
        public = json.loads((OUT / (args.name + '-public-semantic/semantic.json')).read_text())
        assert len(public) == 4 and all(value['identical'] for value in public.values())
        serve_path = OUT / (args.name + '-serve-bounded.json')
        if not serve_path.exists():
            serve_path = OUT / (args.name + '-serve.json')
        serve = json.loads(serve_path.read_text())
        assert len(serve) == 4 and all(row['completed'] for row in serve)
        dependencies = json.loads((OUT / (args.name + '-serve-dependencies.json')).read_text())
        assert len(dependencies) == 2 and all(row['edits'] == 9 and row['complete_observed_and_logical_dependencies_match'] for row in dependencies)
        analysis = json.loads((OUT / (args.name + '-analysis.json')).read_text())
        assert analysis['serve']['no_observed_median_regression'], 'Investigate slower serve cases before accepting'
        expected = manifest['sources'][args.name]
    else:
        expected = manifest['sources'][experiment['control']]
    # Never retain an unverified production delta after resolving an experiment.
    for path, fingerprint in expected.items():
        if path.startswith(('src/', 'crates/', 'modules/', 'tests/')) and path.endswith(('.rs', '.toml')) or path in ['Cargo.toml', 'Cargo.lock']:
            assert common.digest(REPO / path) == fingerprint[1], path
    experiment.update(state=args.outcome, reason=args.reason)
    common.save(OUT / 'manifest.json', manifest)
    print('resolved', args.name, args.outcome, flush=True)


def snapshot(args):
    manifest = guard()
    assert any(e['name'] == args.name and e['state'] == 'started' for e in manifest['experiments'])
    manifest['sources'][args.name] = freeze(OUT / 'sources' / args.name)
    common.save(OUT / 'manifest.json', manifest)


def build(args):
    manifest = guard()
    source = OUT / 'sources' / args.name
    assert common.inventory(source) == manifest['sources'][args.name]
    env = dict(os.environ, CARGO_TARGET_DIR=str(OUT / 'target'), CARGO_BUILD_JOBS='2',
               CARGO_INCREMENTAL='0', CARGO_PROFILE_RELEASE_DEBUG='1')
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=source, env=env))
    packages = [p['name'] for p in metadata['packages'] if p['id'] in metadata['workspace_members']]
    with (OUT / 'logs' / ('build-' + args.name + '.log')).open('wb') as log:
        # Only external, reproducible local build products; retain dependency cache.
        subprocess.run(['cargo', 'clean', '--release', *[flag for package in packages for flag in ['-p', package]]],
                       cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(['cargo', 'build', '--locked', '--release', '-p', 'notist-cli', '--bin', 'notist'],
                       cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    assert common.inventory(source) == manifest['sources'][args.name]
    shutil.copy2(OUT / 'target/release/notist', OUT / 'bin' / args.name)
    manifest['binaries'][args.name] = common.digest(OUT / 'bin' / args.name)
    common.save(OUT / 'manifest.json', manifest)


def measure(args):
    manifest = guard()
    samples = OUT / (args.cohort + '-samples.json')
    assert not samples.exists(), 'Never overwrite a cohort'
    for name in args.kinds:
        assert common.digest(OUT / 'bin' / name) == manifest['binaries'][name]
        assert common.inventory(OUT / 'sources' / name) == manifest['sources'][name]
    expected = json.loads((OUT / 'output-inventory.json').read_text())
    env = dict(os.environ, NO_COLOR='1', TERM='dumb', PYTHONDONTWRITEBYTECODE='1')
    for key in ['LD_PRELOAD', 'RESOURCE_PROFILE', 'RESOURCE_DIAGNOSTIC', 'XIV_PROFILE_REQUESTS', 'EVAL_AB_OBSERVER']:
        assert not env.get(key), key
    rows = []
    for rep in range(args.repetitions):
        for kind in args.kinds if rep % 2 == 0 else args.kinds[::-1]:
            output = OUT / 'site/dist'
            if output.exists():
                shutil.rmtree(output)
            label = f'{args.cohort}-{rep}-{kind}'
            with (OUT / 'logs' / (label + '.stdout')).open('wb') as stdout, (OUT / 'logs' / (label + '.stderr')).open('wb') as stderr:
                started = time.perf_counter()
                child = subprocess.Popen([str(OUT / 'bin' / kind), 'build'], cwd=OUT / 'site', env=env, stdout=stdout, stderr=stderr)
                _, status, usage = os.wait4(child.pid, 0)
                child.returncode = os.waitstatus_to_exitcode(status)
                wall = time.perf_counter() - started
            row = dict(kind=kind, repetition=rep, label=label, wall_s=wall, user_s=usage.ru_utime,
                       system_s=usage.ru_stime, maxrss_bytes=usage.ru_maxrss * 1024,
                       major_faults=usage.ru_majflt, minor_faults=usage.ru_minflt,
                       involuntary_context_switches=usage.ru_nivcsw, exit=child.returncode,
                       load=Path('/proc/loadavg').read_text().strip())
            common.save(OUT / 'logs' / (label + '.json'), row)
            rows.append(row)
            common.save(samples, rows)
            assert child.returncode == 0, row
            row['output_verified'] = common.inventory(OUT / 'site/dist/icexiv-db') == expected
            common.save(samples, rows)
            assert row['output_verified'], row
            print(label, round(wall, 4), row['maxrss_bytes'], flush=True)
    result = {kind: dict(n=len(found := [r for r in rows if r['kind'] == kind]),
                         wall_median_s=statistics.median(r['wall_s'] for r in found),
                         maxrss_bytes=max(r['maxrss_bytes'] for r in found)) for kind in args.kinds}
    common.save(OUT / (args.cohort + '-summary.json'), result)
    guard()
    print(json.dumps(result, indent=2), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'measure', 'begin', 'snapshot', 'build', 'resolve'])
    parser.add_argument('--kinds', nargs='+', default=['r00'])
    parser.add_argument('--repetitions', type=int, default=6)
    parser.add_argument('--cohort', default='baseline')
    parser.add_argument('--name')
    parser.add_argument('--control', default='r00')
    parser.add_argument('--hypothesis')
    parser.add_argument('--outcome', choices=['accepted', 'rejected'])
    parser.add_argument('--reason')
    args = parser.parse_args()
    prepare() if args.action == 'prepare' else globals()[args.action](args)
