#!/usr/bin/env python3
"""Summarize a single matched cohort, never sum cross-batch speedups."""
import json
from pathlib import Path
import random
import statistics

from profile_deadline import OUT
import profile_runtime_cow as common


def analyze(cohort='verified'):
    rows = json.loads((OUT / (cohort + '-samples.json')).read_text())
    kinds = list(dict.fromkeys(row['kind'] for row in rows))
    baseline = {r['round']: r for r in rows if r['kind'] == 'before'}
    result = {}
    for kind in kinds:
        samples = [r for r in rows if r['kind'] == kind]
        assert len(samples) >= (3 if kind == 'before' else 6) and all(r['exit'] == 0 and r['output_verified'] for r in samples)
        median = statistics.median(r['wall_s'] for r in samples)
        result[kind] = dict(n=len(samples), wall_median_s=median,
                            peak_rss_bytes=[r['maxrss_bytes'] for r in samples],
                            largest_peak_rss_bytes=max(r['maxrss_bytes'] for r in samples),
                            wall_target_pass=median <= 3,
                            memory_target_pass=all(r['maxrss_bytes'] <= 300_000_000 for r in samples),
                            wall_gap_s=max(0, median-3),
                            rss_gap_bytes=max(0, max(r['maxrss_bytes'] for r in samples)-300_000_000))
        if kind != 'before':
            gains = [(baseline[r['round']]['wall_s']-r['wall_s'])/baseline[r['round']]['wall_s']*100 for r in samples if r['round'] in baseline]
            rng = random.Random(20261105)
            bootstrap = sorted(statistics.median(rng.choices(gains, k=len(gains))) for _ in range(10000))
            result[kind].update(paired_n=len(gains), paired_gain_percent=gains, paired_median_gain_percent=statistics.median(gains),
                                paired_descriptive_bootstrap_95_percent=[bootstrap[250], bootstrap[9749]])
    common.save(OUT / (cohort + '-analysis.json'), result)
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    import argparse
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cohort', default='verified')
    analyze(parser.parse_args().cohort)
