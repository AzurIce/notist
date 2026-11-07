#!/usr/bin/env python3
"""Separate glibc allocation API traffic; never use this run as latency evidence."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

from profile_kernel_site import guard, inventory, REPO
from profile_icexiv_refresh import digest, environment
from profile_xiv_history import save


def run(args):
    guard(args)
    if os.environ.get("LD_PRELOAD"):
        raise ValueError("start without a preload")
    source = args.out / "allocation-observer.c"
    shutil.copy2(REPO / "tools/evaluator_ab_alloc.c", source)
    shutil.copy2(Path(__file__), args.out / "allocation-reproducer.py")
    library = args.out / "bin/allocation-observer.so"
    command = ["cc", "-O2", "-shared", "-fPIC", str(source), "-o", str(library)]
    subprocess.run(command, check=True)
    expected = json.loads((args.out / "output-inventory.json").read_text())
    proof = dict(compile_command=command, source_sha256=digest(source), library_sha256=digest(library),
                 note="glibc malloc/calloc/realloc/free API events for Rust process only; requested-byte traffic, not live heap, RSS or latency; excludes aligned entrypoints and libc-internal calls", samples=[])
    for index in range(2):
        for kind in (["baseline", "normal"] if index == 0 else ["normal", "baseline"]):
            label = f"alloc-{index}-{kind}"
            binary = args.out / "bin" / kind
            env = dict(environment(), LD_PRELOAD=str(library), EVAL_AB_ALLOC_TARGET=str(binary))
            with (args.out / "logs" / f"{label}.stdout").open("wb") as stdout, (args.out / "logs" / f"{label}.stderr").open("wb") as stderr:
                subprocess.run([str(binary), "build"], cwd=args.out / "site", env=env, stdout=stdout, stderr=stderr, check=True)
            lines = [line for line in (args.out / "logs" / f"{label}.stderr").read_text().splitlines()
                     if line.startswith("[eval-native-alloc] ")]
            assert len(lines) == 1
            values = {key: int(value) for key,value in re.findall(r"(\w+)=(\d+)", lines[0])}
            assert inventory(args.out / "site/dist/icexiv-db") == expected
            proof["samples"].append(dict(kind=kind, round=index, **values))
            save(args.out / "allocations.json", proof)
            print(label, values, flush=True)
    guard(args)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    args.out = args.out.resolve()
    run(args)
