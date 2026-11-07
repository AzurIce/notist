#!/usr/bin/env python3
"""Use the same coarse probes on the frozen pre-refactor baseline, not old timings."""
import argparse
import difflib
import json
import os
from pathlib import Path
import shutil
import subprocess

from profile_kernel_site import ENV, guard, instrument, inventory, process
from profile_icexiv_refresh import digest
from profile_xiv_history import save


def run(args):
    manifest = guard(args)
    source = args.out / "sources/baseline-coarse"
    shutil.copytree(args.out / "sources/baseline", source)
    instrument(source, False)
    patch = []
    for path in sorted(source.rglob("*.rs")):
        relative = path.relative_to(source)
        original = args.out / "sources/baseline" / relative
        before = original.read_text() if original.exists() else ""
        patch.extend(difflib.unified_diff(before.splitlines(True), path.read_text().splitlines(True),
                     fromfile="a/"+str(relative), tofile="b/"+str(relative)))
    (args.out / "baseline-coarse.patch").write_text("".join(patch))
    shutil.copy2(Path(__file__), args.out / "baseline-probe-reproducer.py")
    manifest["sources"]["baseline-coarse"] = inventory(source)
    env = dict(os.environ, **ENV, CARGO_TARGET_DIR=str(args.out / "target"))
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version=1"], cwd=source, env=env))
    packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
    with (args.out / "logs/build-baseline-coarse.log").open("wb") as log:
        subprocess.run(["cargo", "clean", "--release", *[v for p in packages for v in ["-p", p]]], cwd=source, env=env,
                       stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(["cargo", "build", "--release", "--locked", "--offline", "-p", "notist-cli", "--bin", "notist", "--features", "profiling"], cwd=source, env=env,
                       stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(args.out / "target/release/notist", args.out / "bin/baseline-coarse")
    manifest["binaries"]["baseline-coarse"] = digest(args.out / "bin/baseline-coarse")
    save(args.out / "manifest.json", manifest)
    rows = []
    for index in range(2):
        for kind in (["baseline-coarse", "coarse"] if index == 0 else ["coarse", "baseline-coarse"]):
            rows.append(process(args, kind, f"paired-probe-{index}-{kind}"))
            save(args.out / "paired-probe-samples.json", rows)
    guard(args)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    args.out = args.out.resolve()
    run(args)
