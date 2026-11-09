#!/usr/bin/env python3
"""Serial, fingerprinted full-site evaluation concurrency experiment.

Use an external output directory containing bin/baseline and bin/candidate.
prepare copies a prior frozen input, changing only its manifest syntax for the
current package API; measure establishes a NEW baseline output inventory.
Candidates use the unified --jobs option; "serial" means --jobs 1 for both
evaluation and templates. A legacy baseline keeps its original defaults.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import time

from profile_support import digest, inventory, save


def prepare(args):
    root = args.out
    assert not (root / 'site').exists(), 'use a fresh site directory'
    shutil.copytree(args.reference / 'site', root / 'site', symlinks=True,
                    ignore=shutil.ignore_patterns('dist', '__pycache__', 'notist-output'))
    manifest = root / 'site/Notist.toml'
    save(root / 'original-manifest.json', {'text': manifest.read_text()})
    manifest.write_text('''[package]
name = "icexiv-db"
[site]
title = "IceXIV DB"
base_url = "/"
content = "content"
templates = "templates"
static_dir = "static"
output = "dist/icexiv-db"
[site.extensions.items]
command = ["python3", "scripts/items.py", "extension"]
watch = ["scripts", "sources", "inputs/game-export/manifest.json", "inputs/game-export/items.json"]
''')
    if args.event_protocol:
        # The frozen generator predates build-scoped JSON Lines events. Adapt
        # only its transport; keep its validation and document generation intact.
        adapter = root / 'site/scripts/eval_parallel_events.py'
        adapter.write_text('''import json
import sys
from items import ROOT, extend

for line in sys.stdin:
    request = json.loads(line)
    event = request["event"]
    response = {"version": 2, "event": event}
    try:
        if request["version"] != 2:
            raise ValueError("unsupported event protocol")
        if event == "before_compile":
            catalog = json.loads((ROOT / "data/catalog.json").read_text())
            previous = {"version": 1, "stage": "prepare", "input": {
                "documents": request["documents"], "data": {"catalog": catalog}}}
            result = extend(previous, ROOT / "inputs/game-export")
            response["documents"] = result["input"]["documents"]
        elif event != "before_template":
            raise ValueError("unsupported event")
    except Exception as error:
        response["errors"] = [str(error)]
    print(json.dumps(response, ensure_ascii=False, separators=(",", ":")), flush=True)
''')
        manifest.write_text(manifest.read_text().replace(
            '["python3", "scripts/items.py", "extension"]',
            '["python3", "scripts/eval_parallel_events.py"]'))
    save(root / 'inputs.json', inventory(root / 'site'))


def run(root, mode, label, expected):
    log = root / 'logs' / label
    assert not log.with_suffix('.json').exists(), 'never overwrite a measurement'
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', NO_COLOR='1', TERM='dumb')
    for key in ['LD_PRELOAD', 'NOTIST_EVAL_WORKERS', 'NOTIST_EVAL_TIMING']:
        env.pop(key, None)
    binary = root / 'bin' / ('baseline' if mode == 'baseline' else 'candidate')
    if mode != 'baseline':
        env['NOTIST_EVAL_TIMING'] = '1'
    command = [str(binary), 'build']
    if mode != 'baseline':
        command.extend(['--jobs', '1' if mode == 'serial' else mode])
    start = time.perf_counter()
    with log.with_suffix('.stdout').open('xb') as stdout, log.with_suffix('.stderr').open('xb') as stderr:
        child = subprocess.Popen(command, cwd=root / 'site', env=env,
                                 stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    wall = time.perf_counter() - start
    assert child.returncode == 0, (label, log.with_suffix('.stderr'))
    actual = inventory(root / 'site/dist/icexiv-db')
    assert sum(p.endswith('.html') for p in actual) == 51739
    if expected is None:
        save(root / 'output-inventory.json', actual)
    else:
        assert actual == expected, f'full output mismatch: {label}'
    text = log.with_suffix('.stderr').read_text()
    match = re.search(r'\[eval-batch\] seconds=([\d.]+)', text)
    row = dict(mode=mode, label=label, wall_s=wall, user_s=usage.ru_utime,
               system_s=usage.ru_stime, maxrss_kib=usage.ru_maxrss,
               voluntary_switches=usage.ru_nvcsw, involuntary_switches=usage.ru_nivcsw,
               evaluation_s=float(match[1]) if match else None,
               output_identical=True, load=Path('/proc/loadavg').read_text().strip())
    save(log.with_suffix('.json'), row)
    print(json.dumps(row), flush=True)
    return row


def measure(args):
    root = args.out
    assert not (root / 'samples.json').exists()
    binaries = {name: digest(root / 'bin' / name) for name in ['baseline', 'candidate']}
    save(root / 'binaries.json', binaries)
    run(root, 'baseline', 'initial-baseline', None)
    expected = json.loads((root / 'output-inventory.json').read_text())
    modes = ['baseline', 'serial', '2', '4', '8']
    for mode in modes[1:]:
        run(root, mode, f'warmup-{mode}', expected)
    rows = []
    for i in range(args.rounds):
        # Rotate and reverse ordering to avoid always measuring eight workers last.
        order = modes[i % len(modes):] + modes[:i % len(modes)]
        if i % 2:
            order = list(reversed(order))
        for mode in order:
            rows.append(run(root, mode, f'round-{i}-{mode}', expected))
            save(root / 'samples.json', rows)
    fields = ['wall_s', 'user_s', 'system_s', 'maxrss_kib', 'evaluation_s']
    summary = {mode: {f: statistics.median(r[f] for r in rows if r['mode'] == mode)
                      for f in fields if mode != 'baseline' or f != 'evaluation_s'}
               for mode in modes}
    paired = {}
    for mode in modes[1:]:
        ratios = [next(r['wall_s'] for r in rows if r['label'] == f'round-{i}-{mode}') /
                  next(r['wall_s'] for r in rows if r['label'] == f'round-{i}-baseline')
                  for i in range(args.rounds)]
        paired[mode] = dict(ratios=ratios, median_change_percent=100 * (statistics.median(ratios) - 1))
    save(root / 'summary.json', dict(medians=summary, paired_vs_baseline=paired))
    assert {name: digest(root / 'bin' / name) for name in binaries} == binaries
    assert {p: v for p, v in inventory(root / 'site').items() if not p.startswith('dist/')} == json.loads((root / 'inputs.json').read_text())
    print(json.dumps(summary, indent=2), flush=True)


def confirm(args):
    root = args.out
    assert not (root / 'confirmation-samples.json').exists()
    expected = json.loads((root / 'output-inventory.json').read_text())
    binaries = json.loads((root / 'binaries.json').read_text())
    assert {name: digest(root / 'bin' / name) for name in binaries} == binaries
    rows = []
    for mode in ['4', '8']:
        for i in range(args.rounds):
            order = ['serial', mode] if i % 2 == 0 else [mode, 'serial']
            for current in order:
                rows.append(run(root, current, f'confirm-{mode}-{i}-{current}', expected))
                save(root / 'confirmation-samples.json', rows)
    summary = {}
    for mode in ['4', '8']:
        sample = [r for r in rows if r['label'].startswith(f'confirm-{mode}-')]
        summary[mode] = {}
        for field in ['wall_s', 'evaluation_s']:
            ratios = [next(r[field] for r in sample if r['label'] == f'confirm-{mode}-{i}-{mode}') /
                      next(r[field] for r in sample if r['label'] == f'confirm-{mode}-{i}-serial')
                      for i in range(args.rounds)]
            summary[mode][field] = dict(ratios=ratios, median_change_percent=100 * (statistics.median(ratios) - 1))
        summary[mode]['medians'] = {
            current: {field: statistics.median(r[field] for r in sample if r['mode'] == current)
                      for field in ['wall_s', 'evaluation_s', 'user_s', 'system_s', 'maxrss_kib']}
            for current in ['serial', mode]
        }
    save(root / 'confirmation-summary.json', summary)
    assert {name: digest(root / 'bin' / name) for name in binaries} == binaries
    assert {p: v for p, v in inventory(root / 'site').items() if not p.startswith('dist/')} == json.loads((root / 'inputs.json').read_text())
    print(json.dumps(summary, indent=2), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=['prepare', 'measure', 'confirm'])
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--reference', type=Path)
    parser.add_argument('--rounds', type=int, default=5)
    parser.add_argument('--event-protocol', action='store_true', help='adapt frozen generator to build-scoped event protocol')
    args = parser.parse_args()
    globals()[args.action](args)
