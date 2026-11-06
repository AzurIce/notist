#!/usr/bin/env python3
"""Lossless base+delta archive of our disposable edit fixtures, then reclaim inodes.

Never removes canonical inputs, sources, binaries, logs or measurement samples.
Every removed byte is either in the verified full input baseline or saved delta;
derived cold output bytes retain complete per-revision inventories/reproducers.
"""
import argparse
import json
from pathlib import Path
import shutil

import profile_runtime_cow as common
from profile_ten_rounds import OUT, guard


def archive(label):
    baseline = guard()['inputs']
    root = OUT / ('serve-' + label)
    assert root.is_dir()
    current = common.inventory(root)
    destination = OUT / 'fixture-archives' / label
    destination.mkdir(parents=True, exist_ok=False)
    changed = {p: value for p, value in current.items() if baseline.get(p) != value}
    deleted = sorted(baseline.keys() - current.keys())
    for relative, value in changed.items():
        source = root / relative
        target = destination / 'delta' / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(source, target)
        assert [target.stat().st_size, common.digest(target)] == value
    summary = dict(label=label, baseline='../../manifest.json:inputs',
                   changed=changed, deleted=deleted, files=len(current))
    common.save(destination / 'input-delta.json', summary)
    assert common.inventory(root) == current, 'Fixture changed during archival'
    # This is an exact sandbox created by profile_serve_edits, never a live project.
    shutil.rmtree(root)
    cold = OUT / ('cold-' + label)
    if cold.exists():
        inventories = sorted((OUT / 'logs').glob(label + '-*-cold-inventory.json'))
        actual_output = common.inventory(cold)
        matches = [p for p in inventories if json.loads(p.read_text()) == actual_output]
        assert matches, 'Do not remove an unverified/partially produced output'
        summary['cold_output_inventory'] = str(matches[0].relative_to(OUT))
        common.save(destination / 'input-delta.json', summary)
        shutil.rmtree(cold)
    print(label, 'archived full baseline reference +', len(changed), 'delta files; reclaimed sandbox', flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('labels', nargs='+')
    args = parser.parse_args()
    for label in args.labels:
        archive(label)
