#!/usr/bin/env python3
"""Close the <=10-experiment campaign with one final, unmixed paired cohort."""
import argparse
import json
import os
import subprocess

import profile_runtime_cow as common
from profile_ten_rounds import OUT, REPO, guard


def production(path):
    return path.startswith(('src/', 'crates/', 'modules/', 'tests/', '.cargo/')) or path in [
        'Cargo.toml', 'Cargo.lock', 'build.rs', 'rust-toolchain', 'rust-toolchain.toml',
    ]


def finalize(name, cohort):
    manifest = guard()
    assert len(manifest['experiments']) <= 10
    assert all(e['state'] in ['accepted', 'rejected'] for e in manifest['experiments'])
    assert name == 'r00' or any(e['name'] == name and e['state'] == 'accepted' for e in manifest['experiments'])
    expected = {p: v for p, v in manifest['sources'][name].items() if production(p)}
    files = subprocess.check_output(['git', 'ls-files', '-co', '--exclude-standard', '-z'], cwd=REPO).split(b'\0')
    actual = {}
    for raw in sorted(set(files) - {b''}):
        path = os.fsdecode(raw)
        if production(path) and (REPO / path).is_file():
            actual[path] = [(REPO / path).stat().st_size, common.digest(REPO / path)]
    assert actual == expected, 'Final production is not the accepted frozen source'
    assert subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip() == manifest['git_head']
    rows = json.loads((OUT / (cohort + '-samples.json')).read_text())
    assert {r['kind'] for r in rows} == {'r00', name}
    assert all(r['exit'] == 0 and r['output_verified'] for r in rows)
    summary = json.loads((OUT / (cohort + '-summary.json')).read_text())
    assert summary[name]['n'] >= 6 and summary['r00']['n'] >= 6
    for kind in ['r00', name]:
        assert common.digest(OUT / 'bin' / kind) == manifest['binaries'][kind]
        assert common.inventory(OUT / 'sources' / kind) == manifest['sources'][kind]
    before, after = summary['r00'], summary[name]
    accepted = [e['name'] for e in manifest['experiments'] if e['state'] == 'accepted']
    rejected = [e['name'] for e in manifest['experiments'] if e['state'] == 'rejected']
    result = dict(final_version=name, cohort=cohort, summary=summary,
        binary_sha256=manifest['binaries'][name], production_inventory=actual,
        production_changes_from_start=sorted(p for p in actual if actual[p] != manifest['sources']['r00'].get(p)),
        experiments=len(manifest['experiments']), accepted=accepted, rejected=rejected,
        wall_target_s=3.0, rss_target_bytes=300_000_000,
        wall_target_met=after['wall_median_s'] <= 3.0,
        rss_target_met=after['maxrss_bytes'] <= 300_000_000,
        wall_median_reduction_percent=100*(1-after['wall_median_s']/before['wall_median_s']),
        maxrss_reduction_bytes=before['maxrss_bytes']-after['maxrss_bytes'],
        note='Only this final paired cohort reports campaign-wide differences. No summing round gains or mixing historical samples. Full input/output and source fingerprints retained; all tests/semantic/serve/dependency gates belong to the identical frozen final version. No commits or pushes.')
    destination = OUT / 'delivery-final.json'
    assert not destination.exists()
    common.save(destination, result)
    manifest['delivery'] = dict(version=name, cohort=cohort,
        summary_sha256=common.digest(destination), targets_met=result['wall_target_met'] and result['rss_target_met'])
    common.save(OUT / 'manifest.json', manifest)
    print(json.dumps({k: v for k, v in result.items() if k != 'production_inventory'}, indent=2))


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--name', required=True)
    p.add_argument('--cohort', default='delivery')
    args = p.parse_args()
    finalize(args.name, args.cohort)
