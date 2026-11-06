#!/usr/bin/env python3
"""Regression, full semantic/dependency and actual edit-sequence validation.

Run only outside ordinary timing cohorts. Existing historical tools stay intact.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import types

import profile_runtime_cow as common
import profile_ten_semantic as semantic
import profile_serve_edits as edits
import archive_ten_fixtures as fixture_archive
from profile_ten_rounds import OUT, guard


def tests(name, control):
    manifest = guard()
    source = OUT / 'sources' / name
    assert common.inventory(source) == manifest['sources'][name]
    (source / 'target').mkdir(exist_ok=True)
    env = dict(os.environ, CARGO_TARGET_DIR=str(OUT / 'target'), CARGO_BUILD_JOBS='2',
               CARGO_INCREMENTAL='0', CARGO_PROFILE_RELEASE_DEBUG='1')
    modes = [ ('default', ['--workspace', '--lib', '--tests']),
              ('all', ['--workspace', '--lib', '--tests', '--all-features']),
              ('no-default', ['-p', 'notist', '-p', 'notist-core', '-p', 'notist-pipeline', '--lib', '--tests', '--no-default-features']) ]
    results = []
    result_path = OUT / (name + '-tests.json')
    assert not result_path.exists()
    for label, flags in modes:
        log_path = OUT / 'logs' / (name + '-tests-' + label + '.log')
        with log_path.open('wb') as log:
            result = subprocess.run(['cargo', 'test', '--locked', '--release', *flags, '--', '--test-threads=1'],
                                    cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT)
        text = log_path.read_text()
        results.append(dict(mode=label, exit=result.returncode,
                            passed=sum(map(int, re.findall(r'test result: ok\. (\d+) passed', text))),
                            targets=text.count('test result:'), warnings=text.count('warning:')))
        common.save(result_path, results)
        print(results[-1], flush=True)
        assert result.returncode == 0
    assert common.inventory(source) == manifest['sources'][name]
    fixture = OUT / (name + '-public-semantic')
    fixture.mkdir()
    (fixture / 'bin').mkdir()
    (fixture / 'logs').mkdir()
    common.KINDS = [control, name]
    for kind in common.KINDS:
        (fixture / 'bin' / kind).symlink_to(OUT / 'bin' / kind)
    common.semantic(types.SimpleNamespace(out=fixture))


def full_semantic(name, control):
    for kind in [control, name]:
        summary = OUT / ('semantic-' + kind + '-summary.json')
        if summary.exists():
            # An accepted version may already have complete diagnostic evidence.
            # Reuse only the identical immutable source, binary and full stream;
            # do not overwrite prior runs or use their times as a new cohort.
            manifest = guard()
            assert common.inventory(OUT / 'sources' / kind) == manifest['sources'][kind]
            proof = json.loads(summary.read_text())
            assert proof['frames'] == 2 * 51_739 + 1 and proof['output_verified']
            assert common.digest(OUT / 'bin' / ('semantic-' + kind)) == proof['binary_sha256']
            assert common.digest(OUT / ('semantic-' + kind + '.jsonl')) == proof['evidence_sha256']
            assert common.inventory(OUT / 'sources' / ('semantic-' + kind)) == json.loads((OUT / ('semantic-' + kind + '-sources.json')).read_text())
        else:
            semantic.prepare(kind)
            semantic.build(kind)
            semantic.run(kind)
    before, after = [OUT / ('semantic-' + kind + '.jsonl') for kind in [control, name]]
    assert common.digest(before) == common.digest(after), 'Full-site semantic/dependency fingerprints differ'
    common.save(OUT / (name + '-full-semantic.json'), dict(control=control, candidate=name,
                frames=2*51_739+1, identical=True, evidence_sha256=common.digest(before)))


def observed_frames(path):
    frames = []
    with path.open('rb') as log:
        for line in log:
            if line.startswith(b'[semantic-page] '):
                kind, label = json.loads(line.removeprefix(b'[semantic-page] '))
                assert (kind, label) == ('observed', 'build')
                payload = log.readline()
                assert payload.startswith(b'[semantic-json] ')
                frames.append(json.loads(payload.removeprefix(b'[semantic-json] ')))
    return frames


def serve(name, control):
    edits.OUT = OUT
    # Balanced process order; cold oracles are always outside each watched root.
    completed = []
    for block, kinds in enumerate([[control, name], [name, control]]):
        for kind in kinds:
            label = f'{name}-serve-{block}-{kind}'
            rows = edits.run(kind, label)
            assert len(rows) == 9
            assert all(r.get('full_http_hashes_match') or r.get('diagnostics_match') for r in rows)
            for row in rows:
                report = row['event']['report']
                if row['edit'] == 'error':
                    actual = (OUT / 'logs' / f'{label}-error-cold.stderr').read_text()
                    assert ''.join(issue['detail'] for issue in report['errors']) == actual
                elif row['edit'] != 'initial':
                    assert row['event']['observed_edit_to_event_s'] >= report['elapsed_ms']/1000
            completed.append(dict(kind=kind, block=block, label=label, completed=True))
            common.save(OUT / (name + '-serve.json'), completed)


def serve_bounded(name, control):
    edits.OUT = OUT
    destination = OUT / (name + '-serve-bounded.json')
    assert not destination.exists(), 'Keep the failed/unbalanced cohort separate'
    completed = []
    for block, kinds in enumerate([[control, name], [name, control]]):
        for kind in kinds:
            label = f'{name}-bounded-{block}-{kind}'
            rows = edits.run(kind, label)
            assert len(rows) == 9
            assert all(r.get('full_http_hashes_match') or r.get('diagnostics_match') for r in rows)
            for row in rows:
                report = row['event']['report']
                if row['edit'] == 'error':
                    actual = (OUT / 'logs' / f'{label}-error-cold.stderr').read_text()
                    assert ''.join(issue['detail'] for issue in report['errors']) == actual
                elif row['edit'] != 'initial':
                    assert row['event']['observed_edit_to_event_s'] >= report['elapsed_ms']/1000
            completed.append(dict(kind=kind, block=block, label=label, completed=True,
                fixture_policy='Independent ordinary filesystem copy; after process exit archive verified base+delta and derived output inventories, then reclaim the disposable directories'))
            common.save(destination, completed)
            fixture_archive.archive(label)


def serve_dependencies(name, control):
    edits.OUT = OUT
    # Instrumented diagnostic only. Timings in these rows are never used for gains.
    os.environ['NOTIST_SEMANTIC_ONLY_OBSERVED'] = '1'
    proofs = []
    try:
        for kind in [control, name]:
            label = f'{name}-serve-dependencies-{kind}'
            rows = edits.run('semantic-' + kind, label)
            server = observed_frames(OUT / 'logs' / (label + '-serve.stderr'))
            assert len(server) == len(rows) == 9
            for dependencies, row in zip(server, rows):
                cold = observed_frames(OUT / 'logs' / f'{label}-{row["edit"]}-cold.stderr')
                assert cold == [dependencies], (kind, row['edit'])
            proofs.append(dict(kind=kind, edits=9, complete_observed_and_logical_dependencies_match=True))
            common.save(OUT / (name + '-serve-dependencies.json'), proofs)
            fixture_archive.archive(label)
    finally:
        os.environ.pop('NOTIST_SEMANTIC_ONLY_OBSERVED', None)


def error_repeat(name, control):
    edits.OUT = OUT
    destination = OUT / (name + '-error-repeat.json')
    assert not destination.exists()
    completed = []
    # Independent fresh-process repeat of the exact initial -> edit -> error
    # prefix, not a warmed synthetic failure or a CLI-minus-publication estimate.
    for block in range(3):
        for kind in [control, name] if block % 2 == 0 else [name, control]:
            label = f'{name}-error-repeat-{block}-{kind}'
            rows = edits.run(kind, label, stop_after_error=True, measure_cpu=True)
            assert [r['edit'] for r in rows] == ['initial', 'content-edit', 'error']
            row = rows[-1]
            assert [issue['message'] for issue in row['event']['report']['errors']] == [
                'missing embedded resource `deadline-missing.png`',
                'local embed requires a host-provided resource URL',
            ]
            assert row['event']['report']['pages'] == 51_739 and row['event']['revision'] == 2
            actual = (OUT / 'logs' / (label + '-error-cold.stderr')).read_text()
            assert ''.join(issue['detail'] for issue in row['event']['report']['errors']) == actual
            completed.append(dict(kind=kind, block=block, label=label, completed=True))
            common.save(destination, completed)
            fixture_archive.archive(label)


def serve_repeat(name, control):
    edits.OUT = OUT
    destination = OUT / (name + '-serve-repeat.json')
    assert not destination.exists()
    completed = []
    for block in range(3):
        for kind in [control, name] if block % 2 == 0 else [name, control]:
            label = f'{name}-serve-repeat-{block}-{kind}'
            rows = edits.run(kind, label, measure_cpu=True)
            assert len(rows) == 9
            assert all(r.get('full_http_hashes_match') or r.get('diagnostics_match') for r in rows)
            error = next(r for r in rows if r['edit'] == 'error')
            report = error['event']['report']
            assert [issue['message'] for issue in report['errors']] == [
                'missing embedded resource `deadline-missing.png`',
                'local embed requires a host-provided resource URL',
            ]
            assert report['pages'] == 51_739 and error['event']['revision'] == 2
            actual = (OUT / 'logs' / (label + '-error-cold.stderr')).read_text()
            assert ''.join(issue['detail'] for issue in report['errors']) == actual
            completed.append(dict(kind=kind, block=block, label=label, completed=True))
            common.save(destination, completed)
            fixture_archive.archive(label)


def resume_bounded(name, control):
    """Resume one interrupted final process; never replace completed samples."""
    edits.OUT = OUT
    destination = OUT / (name + '-serve-bounded.json')
    completed = json.loads(destination.read_text())
    assert [(r['block'], r['kind']) for r in completed] == [(0, control), (0, name), (1, name)]
    assert all(r['completed'] for r in completed)
    interrupted = f'{name}-bounded-1-{control}'
    # Caller has established the interrupted process no longer exists. Retain
    # its partial rows and logs; base+delta archive only the disposable fixture.
    fixture_archive.archive(interrupted)
    label = f'{name}-bounded-recovery-1-{control}'
    resumed_at = subprocess.check_output(['date', '-Ins'], text=True).strip()
    rows = edits.run(control, label)
    assert len(rows) == 9
    assert all(r.get('full_http_hashes_match') or r.get('diagnostics_match') for r in rows)
    for row in rows:
        report = row['event']['report']
        if row['edit'] == 'error':
            actual = (OUT / 'logs' / f'{label}-error-cold.stderr').read_text()
            assert ''.join(issue['detail'] for issue in report['errors']) == actual
        elif row['edit'] != 'initial':
            assert row['event']['observed_edit_to_event_s'] >= report['elapsed_ms']/1000
    completed.append(dict(kind=control, block=1, label=label, completed=True,
        interrupted_attempt=interrupted, resumed_at=resumed_at,
        note='Fourth AB/BA process restarted fresh after harness interruption; original partial rows/logs retained and excluded, completed samples never replaced. Timing gap recorded, not a new optimization experiment.'))
    common.save(destination, completed)
    fixture_archive.archive(label)


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('action', choices=['tests', 'full_semantic', 'serve', 'serve_bounded', 'serve_dependencies', 'error_repeat', 'serve_repeat', 'resume_bounded'])
    p.add_argument('--name', required=True)
    p.add_argument('--control', default='r00')
    args = p.parse_args()
    globals()[args.action](args.name, args.control)
