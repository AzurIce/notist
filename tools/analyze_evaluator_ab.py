#!/usr/bin/env python3
"""Aggregate independent A/B evidence without conflating its measurement modes."""
import argparse
import json
from pathlib import Path
import re
import statistics

from profile_xiv_history import save


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("out", type=Path)
    args = p.parse_args()
    out = args.out
    rows = json.loads((out / "diagnostics.json").read_text())
    counts, allocations, sampled = {}, {}, {}
    for kind in ["base-probe", "combined-probe"]:
        counted = next(r for r in rows if r["kind"] == kind and r.get("mode") == "count")
        allocated = next(r for r in rows if r["kind"] == kind and r.get("mode") == "alloc")
        names = set().union(*(v.keys() for v in counted["observer"]["metrics"].values()))
        counts[kind] = {name: sum(v.get(name, {}).get("calls", 0) for v in counted["observer"]["metrics"].values()) for name in sorted(names)}
        allocations[kind] = {}
        names = set().union(*(v.keys() for v in allocated["observer"]["metrics"].values()))
        for name in names:
            allocations[kind][name] = {field: sum(v.get(name, {}).get(field, 0) for v in allocated["observer"]["metrics"].values()) for field in ["allocs", "allocated_bytes", "frees", "freed_bytes", "reallocs"]}
        times = [r for r in rows if r["kind"] == kind and r.get("mode") == "time"]
        sampled[kind] = {}
        for page_kind in ["item", "catalog", "home"]:
            selected = times[0]["observer"]["pages"][page_kind]["selected"]
            names = set().union(*(r["observer"]["metrics"][page_kind] for r in times))
            sampled[kind][page_kind] = {
                "seen": times[0]["observer"]["pages"][page_kind]["seen"], "selected_per_run": selected,
                "mean_seconds": {name: {field.replace("_ns", "_s"): statistics.mean(r["observer"]["metrics"][page_kind].get(name, {}).get(field, 0) / 1e9 for r in times) for field in ["inclusive_ns", "self_ns"]} for name in names},
            }
    for invariant in ["final_nodes", "native_calls", "code_calls", "read_data", "data_parse"]:
        assert counts["base-probe"][invariant] == counts["combined-probe"][invariant], invariant
    stage_rows = [r for r in rows if r.get("mode") == "off"]
    stages = {r["kind"]: {name: v["seconds"] for name, v in r["stages"].items()} for r in stage_rows}
    rss = {}
    for row in rows:
        if "peak_observed_rss_kib" in row:
            rss.setdefault(row["kind"], []).append(row["peak_observed_rss_kib"])
    tests = {}
    for path in (out / "logs").glob("tests-*.log"):
        if "attempt" in path.name:
            continue
        passed = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed", path.read_text())
        tests[path.stem] = {"groups": len(passed), "passed": sum(int(a) for a, _ in passed), "failed": sum(int(b) for _, b in passed)}
    result = {"counts": counts, "allocations": allocations, "sampled_wall": sampled, "coarse_off_seconds": stages, "rss_kib": rss, "tests": tests}
    save(out / "analysis.json", result)
    print("wrote", out / "analysis.json")


if __name__ == "__main__":
    main()
