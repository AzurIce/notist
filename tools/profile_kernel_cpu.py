#!/usr/bin/env python3
"""Independent signal-based CPU samples of the frozen clean site binary.

Uses gperftools libprofiler/libunwind when perf_event sampling is unavailable.
No tcmalloc is loaded and child extension programs are excluded by executable
identity. Profile timings are diagnostic, never a clean latency measurement.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

from profile_kernel_site import guard, process, REPO


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(args):
    guard(args)
    if os.environ.get("LD_PRELOAD") or os.environ.get("CPUPROFILE"):
        raise ValueError("start without a profiler or allocator preload")
    reproducer = args.out / "reproducer-cpu"
    reproducer.mkdir()
    for name in ["profile_kernel_cpu.py", "kernel_cpu_probe.c", "profile_kernel_site.py"]:
        shutil.copy2(REPO / "tools" / name, reproducer / name)
    library = args.out / "bin/kernel-cpu-probe.so"
    command = ["cc", "-O2", "-shared", "-fPIC", str(reproducer / "kernel_cpu_probe.c"),
               str(args.profiler), "-Wl,-rpath," + str(args.profiler.parent), "-o", str(library)]
    subprocess.run(command, check=True)
    proof = dict(profiler=str(args.profiler), profiler_sha256=digest(args.profiler),
                 pprof=str(args.pprof), pprof_sha256=digest(args.pprof),
                 library_sha256=digest(library), compile_command=command,
                 reproducer={p.name: digest(p) for p in reproducer.iterdir()}, samples=[],
                 method="gperftools ITIMER_PROF/SIGPROF CPU samples at 199 Hz, libunwind, target Rust process only; no allocator replacement; includes Rust workers, excludes Python child and IO waiting")
    for index in range(2):
        profile = args.out / f"cpu-{index}.prof"
        row = process(args, "normal", f"cpu-userspace-{index}", extra_env={
            "LD_PRELOAD": str(library), "KERNEL_CPU_TARGET": str((args.out / "bin/normal").resolve()),
            "KERNEL_CPU_PROFILE": str(profile), "CPUPROFILE_FREQUENCY": "199"})
        stderr = (args.out / "logs" / f"cpu-userspace-{index}.stderr").read_text()
        assert stderr.count("[kernel-cpu] started=1") == 1
        assert profile.stat().st_size > 128
        for kind, options in [("self", ["-top"]), ("cumulative", ["-top", "-cum"]),
                              ("tree", ["-tree"]), ("raw", ["-raw"])]:
            with (args.out / f"cpu-{index}-{kind}.txt").open("wb") as log:
                subprocess.run([str(args.pprof), *options, "-nodecount=100", "-symbolize=local",
                                str(args.out / "bin/normal"), str(profile)],
                               stdout=log, stderr=subprocess.STDOUT, check=True)
        proof["samples"].append(dict(run=row, profile_sha256=digest(profile)))
        (args.out / "cpu-proof.json").write_text(json.dumps(proof, indent=2) + "\n")
    guard(args)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--profiler", required=True, type=Path)
    parser.add_argument("--pprof", required=True, type=Path)
    args = parser.parse_args()
    args.out = args.out.resolve()
    run(args)
