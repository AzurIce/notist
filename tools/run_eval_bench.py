#!/usr/bin/env python3
"""Freeze evidence, build clean/diagnostic binaries, and run evaluator scenarios.

No production optimization, disk IO workload, fixture reduction or cache result
reuse is performed. Allocation diagnostics never supply performance timings.
"""
import argparse
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import time

from profile_support import BUILD_ENV, digest, save, target_directory

REPO = Path(__file__).resolve().parents[1]


def git(*args):
    return subprocess.check_output(["git", *args], cwd=REPO)


def sources():
    names = set(git("ls-files", "-co", "--exclude-standard", "-z").split(b"\0")) - {b""}
    return {os.fsdecode(name): digest(REPO / os.fsdecode(name)) for name in sorted(names)
            if (REPO / os.fsdecode(name)).is_file()}


def summarize(timing_runs, allocation):
    if not timing_runs:
        raise ValueError("no timing runs")
    keys = [row["id"] for row in timing_runs[0]["rows"]]
    diagnostic = {row["id"]: row for row in allocation["rows"]}
    if allocation["mode"] != "alloc" or not allocation["allocation_instrumented"]:
        raise ValueError("allocation evidence must use the diagnostic binary")
    for run in timing_runs:
        if run["mode"] != "time" or run["allocation_instrumented"] or run["debug_assertions"]:
            raise ValueError("performance evidence must use the uninstrumented release binary")
        if [row["id"] for row in run["rows"]] != keys:
            raise ValueError("scenario identities differ between runs")
    if set(diagnostic) != set(keys):
        raise ValueError("allocation scenarios differ from timing scenarios")
    result = []
    for i, key in enumerate(keys):
        reference = timing_runs[0]["rows"][i]
        related = [run["rows"][i] for run in timing_runs] + [diagnostic[key]]
        for row in related:
            for field in ["facts", "module_source", "document_source"]:
                if row[field] != reference[field]:
                    raise ValueError(f"{key}: fixture or semantics differ in {field}")
        times = [run["rows"][i]["timing"]["median_ns_per_eval"] for run in timing_runs]
        alloc = diagnostic[key]["samples"]
        result.append({
            "id": key, "kind": reference["kind"], "size": reference["size"],
            "facts": reference["facts"], "process_medians_ns": times,
            "median_ns": statistics.median(times), "min_process_median_ns": min(times),
            "max_process_median_ns": max(times),
            "allocation_medians": {name: statistics.median(row[name] for row in alloc) for name in alloc[0]},
        })
    return result


def build_variant(source, out, mode):
    if mode not in {"time", "alloc"}:
        raise ValueError("unknown benchmark build mode: " + mode)
    target = target_directory(out / "target", mode)
    env = dict(os.environ, **BUILD_ENV, CARGO_TARGET_DIR=str(target))
    command = ["cargo", "build", "--release", "--locked", "--offline", "-p", "notist-eval-bench", "--bin", "notist-eval-bench"]
    if mode == "alloc":
        command += ["--features", "alloc-metrics"]
    with (out / "logs" / f"build-{mode}.log").open("wb") as log:
        subprocess.run(command, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(target / "release/notist-eval-bench", out / "bin" / mode)
    return env


def run(args):
    out = args.out.resolve()
    if out.exists() or out.is_relative_to(REPO):
        raise ValueError("use a new output directory outside the checkout")
    if args.runs < 1 or args.runs > 20:
        raise ValueError("runs must be 1..20")
    out.mkdir(parents=True)
    for name in ["bin", "logs", "source"]:
        (out / name).mkdir()
    manifest = {
        "revision": git("rev-parse", "HEAD").decode().strip(),
        "status": git("status", "--short").decode(),
        "sources": sources(), "build_env": BUILD_ENV,
        "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
        "system": platform.platform(), "cpu_affinity": sorted(os.sched_getaffinity(0)),
        "cpuinfo": Path("/proc/cpuinfo").read_text(),
        "additional_rustflags": os.environ.get("RUSTFLAGS", ""),
        "arguments": {key: str(value) if isinstance(value, Path) else value for key, value in vars(args).items()},
        "started_at": time.time(), "note": "one process per timing run; warm in-memory fixtures; CPU clocks/background load uncontrolled; eval plus result Drop, no disk/HTML; allocation runs are separate",
    }
    save(out / "manifest.json", manifest)
    (out / "workspace.patch").write_bytes(git("diff", "--binary", "HEAD"))
    for relative, expected in manifest["sources"].items():
        destination = out / "source" / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(REPO / relative, destination)
        if digest(destination) != expected:
            raise ValueError(f"source changed during freeze: {relative}")
    for mode in ["time", "alloc"]:
        env = build_variant(out / "source", out, mode)
    manifest["binaries"] = {mode: digest(out / "bin" / mode) for mode in ["time", "alloc"]}
    features = subprocess.check_output(["cargo", "tree", "-p", "notist-eval-bench", "--edges", "normal", "--format", "{p} features={f}", "--locked", "--offline"], cwd=out / "source", env=env, text=True)
    if "test-support" in features:
        raise ValueError("benchmark dependency graph enabled test-support")
    (out / "dependency-features.txt").write_text(features)
    save(out / "manifest.json", manifest)
    timing_runs = []
    for i in range(args.runs + 1):
        mode = "time" if i < args.runs else "alloc"
        path = out / (f"time-{i}.json" if mode == "time" else "allocation.json")
        command = [str(out / "bin" / mode), "--mode", mode, "--sizes", args.sizes,
                   "--samples", str(args.samples if mode == "time" else 3),
                   "--sample-ms", str(args.sample_ms), "--seed", str(20261107 + i),
                   "--json", str(path)]
        if args.filter:
            command += ["--filter", args.filter]
        with (out / "logs" / f"{path.stem}.stderr").open("wb") as log:
            subprocess.run(command, cwd=REPO, env=env, stdout=log, stderr=log, check=True)
        result = json.loads(path.read_text())
        if mode == "time":
            timing_runs.append(result)
        else:
            allocation = result
        print(f"completed {path.name}: {len(result['rows'])} scenarios", flush=True)
    summary = summarize(timing_runs, allocation)
    save(out / "summary.json", summary)
    if sources() != manifest["sources"]:
        raise ValueError("working source changed during measurement; results not finalized")
    if any(digest(out / "bin" / mode) != sha for mode, sha in manifest["binaries"].items()):
        raise ValueError("benchmark binary changed during measurement")
    lines = ["# Evaluator scenario benchmarks", "", "Times are the median of fresh-process medians, in microseconds per evaluation **including result destruction**. Allocation counts/bytes are from a separate instrumented binary. Different scenarios may preserve different identities and node counts; they are not interchangeable implementations.", "", "| Case | µs/eval | nodes | allocations | reallocations | requested bytes | peak extra bytes |", "|---|---:|---:|---:|---:|---:|---:|"]
    for row in summary:
        allocation = row["allocation_medians"]
        lines.append(f"| {row['id']} | {row['median_ns']/1000:.3f} | {row['facts']['output_nodes']} | {allocation['allocations']:.0f} | {allocation['reallocations']:.0f} | {allocation['allocation_bytes'] + allocation['reallocation_requested_bytes']:.0f} | {allocation['peak_extra_requested_bytes']:.0f} |")
    (out / "report.md").write_text("\n".join(lines) + "\n")
    manifest["finished_at"] = time.time()
    manifest["integrity_verified"] = True
    manifest["results"] = {p.name: digest(p) for p in out.glob("*.json") if p.name != "manifest.json"}
    save(out / "manifest.json", manifest)
    print(out / "report.md", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--samples", type=int, default=7)
    parser.add_argument("--sample-ms", type=int, default=40)
    parser.add_argument("--sizes", default="1,16,128")
    parser.add_argument("--filter", default="")
    run(parser.parse_args())
