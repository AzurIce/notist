#!/usr/bin/env python3
"""Archive final tools/workspace and fingerprint retained evidence, without cleanup."""
import os
from pathlib import Path
import shutil
import subprocess

import profile_runtime_cow as common
from profile_ten_rounds import OUT, REPO, freeze, guard


def main():
    manifest = guard()
    assert len(manifest['experiments']) == 10
    assert all(e['state'] in ['accepted', 'rejected'] for e in manifest['experiments'])
    assert manifest['delivery']['version'] == 'r04'
    assert common.digest(OUT / 'delivery-final.json') == manifest['delivery']['summary_sha256']
    destination = OUT / 'sources/delivery-workspace'
    inventory = freeze(destination)
    common.save(OUT / 'delivery-workspace-inventory.json', inventory)
    reproducer = OUT / 'reproducer/delivery'
    reproducer.mkdir()
    for source in sorted((REPO / 'tools').glob('*ten*.py')):
        shutil.copy2(source, reproducer / source.name)
    for name in ['profile_runtime_cow.py', 'profile_serve_edits.py', 'tree_residency_probe.rs', 'semantic_tree_probe.rs']:
        shutil.copy2(REPO / 'tools' / name, reproducer / name)
    shutil.copy2(REPO / 'docs/_logs/2026-11-05-ten-rounds.md', reproducer / 'report.md')
    common.save(OUT / 'delivery-reproducer-inventory.json', common.inventory(reproducer))
    with (OUT / 'delivery-workspace.patch').open('xb') as log:
        subprocess.run(['git', 'diff', '--binary'], cwd=REPO, stdout=log, check=True)
    (OUT / 'delivery-git-status.txt').write_text(subprocess.check_output(['git', 'status', '--short'], cwd=REPO, text=True))
    manifest['delivery'].update(
        workspace_inventory_sha256=common.digest(OUT / 'delivery-workspace-inventory.json'),
        reproducer_inventory_sha256=common.digest(OUT / 'delivery-reproducer-inventory.json'))
    common.save(OUT / 'manifest.json', manifest)
    # Canonical input bytes have their complete inventory in manifest.inputs.
    # Cargo products are reproducible caches, not evidence. Nothing is deleted.
    retained = {}
    for directory, dirs, files in os.walk(OUT):
        if Path(directory) == OUT:
            dirs[:] = [d for d in dirs if d not in ['site', 'target', 'target-probe']]
        for name in sorted(files):
            path = Path(directory) / name
            if name == 'delivery-evidence-inventory.json' or '__pycache__' in path.parts:
                continue
            retained[str(path.relative_to(OUT))] = [path.stat().st_size, common.digest(path)]
    common.save(OUT / 'delivery-evidence-inventory.json', dict(files=retained,
        excluded='Canonical site inputs use manifest.inputs; Cargo target caches and Python bytecode excluded; this inventory excludes itself. No files removed.'))
    print('Archived final workspace', len(inventory), 'files and fingerprinted', len(retained), 'retained evidence files')


if __name__ == '__main__':
    main()
