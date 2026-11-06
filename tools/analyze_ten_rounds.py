#!/usr/bin/env python3
"""Descriptive within-cohort analysis; never adds gains from different batches."""
import argparse
import json
import random
import statistics

import profile_runtime_cow as common
from profile_ten_rounds import OUT


def build_analysis(name, control):
    rows = json.loads((OUT / (name + '-ab-samples.json')).read_text())
    baseline = {r['repetition']: r for r in rows if r['kind'] == control}
    candidate = [r for r in rows if r['kind'] == name]
    assert len(baseline) >= 6 and len(candidate) >= 6
    assert all(r['exit'] == 0 and r['output_verified'] for r in rows)
    gains = [100*(baseline[r['repetition']]['wall_s']-r['wall_s'])/baseline[r['repetition']]['wall_s'] for r in candidate]
    rng = random.Random(20261105)
    bootstrap = sorted(statistics.median(rng.choices(gains, k=len(gains))) for _ in range(10000))
    return dict(pairs=len(gains), paired_gain_percent=gains,
                paired_median_gain_percent=statistics.median(gains), faster_pairs=sum(g > 0 for g in gains),
                descriptive_bootstrap_median_95_percent=[bootstrap[250], bootstrap[9749]],
                note='Descriptive six-pair interval, not proof of generality; all slow samples retained')


def serve_analysis(name, control):
    path = OUT / (name + '-serve-bounded.json')
    if not path.exists():
        path = OUT / (name + '-serve.json')
    runs = json.loads(path.read_text())
    assert len(runs) == 4 and all(run['completed'] for run in runs)
    by_kind = {kind: {} for kind in [control, name]}
    for run in runs:
        rows = json.loads((OUT / (run['label'] + '-serve-samples.json')).read_text())
        assert len(rows) == 9
        for row in rows:
            by_kind[run['kind']].setdefault(row['edit'], []).append(row)
    result, regressions = {}, []
    for edit in by_kind[control]:
        before = by_kind[control][edit]
        after = by_kind[name][edit]
        assert len(before) == len(after) == 2
        summary = {}
        for field in ['build_ms', 'edit_to_event_s']:
            def value(row):
                return row['event']['report']['elapsed_ms'] if field == 'build_ms' else row['event']['observed_edit_to_event_s']
            baseline = [value(row) for row in before]
            candidate = [value(row) for row in after]
            old, new = statistics.median(baseline), statistics.median(candidate)
            summary[field] = dict(control_samples=baseline, candidate_samples=candidate,
                                  control_median=old, candidate_median=new,
                                  paired_gain_percent=[100*(a-b)/a for a, b in zip(baseline, candidate)])
            if new > old:
                regressions.append(dict(edit=edit, metric=field, control=old, candidate=new))
        result[edit] = summary
    unresolved = list(regressions)
    repeat = None
    path = OUT / (name + '-error-repeat.json')
    if path.exists():
        runs = json.loads(path.read_text())
        assert len(runs) == 6 and all(r['completed'] for r in runs)
        by_kind = {kind: [] for kind in [control, name]}
        for run in runs:
            rows = json.loads((OUT / (run['label'] + '-serve-samples.json')).read_text())
            assert len(rows) == 3 and rows[-1]['edit'] == 'error'
            by_kind[run['kind']].append(rows[-1]['event'])
        repeat = {}
        for metric in ['build_ms', 'edit_to_event_s', 'cpu_s']:
            def value(event):
                if metric == 'build_ms':
                    return event['report']['elapsed_ms']
                return event['observed_edit_to_event_s'] if metric == 'edit_to_event_s' else event['observed_process_and_children_cpu_s']
            before = [value(event) for event in by_kind[control]]
            after = [value(event) for event in by_kind[name]]
            repeat[metric] = dict(control=before, candidate=after,
                                  control_median=statistics.median(before), candidate_median=statistics.median(after))
        if all(repeat[k]['candidate_median'] <= repeat[k]['control_median'] for k in ['build_ms', 'edit_to_event_s']):
            unresolved = [r for r in unresolved if r['edit'] != 'error']
    full_repeat = None
    path = OUT / (name + '-serve-repeat.json')
    if path.exists():
        runs = json.loads(path.read_text())
        assert len(runs) == 6 and all(r['completed'] for r in runs)
        repeated = {kind: {} for kind in [control, name]}
        for run in runs:
            rows = json.loads((OUT / (run['label'] + '-serve-samples.json')).read_text())
            assert len(rows) == 9
            for row in rows:
                repeated[run['kind']].setdefault(row['edit'], []).append(row['event'])
        full_repeat = {}
        for edit in result:
            metrics = {}
            for metric in ['build_ms', 'edit_to_event_s', 'cpu_s']:
                def value(event):
                    if metric == 'build_ms':
                        return event['report']['elapsed_ms']
                    return event['observed_edit_to_event_s'] if metric == 'edit_to_event_s' else event['observed_process_and_children_cpu_s']
                before = [value(event) for event in repeated[control][edit]]
                after = [value(event) for event in repeated[name][edit]]
                assert len(before) == len(after) == 3
                metrics[metric] = dict(control=before, candidate=after,
                    control_median=statistics.median(before), candidate_median=statistics.median(after))
            full_repeat[edit] = metrics
        unresolved = [r for r in unresolved if
            full_repeat[r['edit']][r['metric']]['candidate_median'] >
            full_repeat[r['edit']][r['metric']]['control_median']]
    return dict(edits=result, needs_repetition=regressions, independent_error_repetition=repeat,
                independent_full_repetition=full_repeat,
                unresolved_needs_repetition=unresolved,
                no_observed_median_regression=not unresolved,
                note='Two ordinary processes per version in AB/BA order; instrumented dependency runs excluded. A slower median requires investigation/repetition, not automatic acceptance.')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--name', required=True)
    p.add_argument('--control', default='r00')
    args = p.parse_args()
    result = dict(build=build_analysis(args.name, args.control), serve=serve_analysis(args.name, args.control))
    common.save(OUT / (args.name + '-analysis.json'), result)
    print(json.dumps(result, indent=2))
