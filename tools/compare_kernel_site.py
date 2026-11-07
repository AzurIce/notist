#!/usr/bin/env python3
"""Same frozen site, interleaved clean CLI A/B across the Item/kernel refactor."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from profile_kernel_site import ENV, guard, inventory, REPO
from profile_icexiv_refresh import digest, environment
from profile_xiv_history import extract, save


def run(args):
    manifest = guard(args)
    source = args.out / "sources/baseline"
    extract(REPO, args.baseline, source)
    manifest["baseline_revision"] = subprocess.check_output(["git", "rev-parse", args.baseline], cwd=REPO, text=True).strip()
    manifest["sources"]["baseline"] = inventory(source)
    shutil.copy2(Path(__file__), args.out / "comparison-reproducer.py")
    env = dict(os.environ, **ENV, CARGO_TARGET_DIR=str(args.out / "target"))
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version=1"], cwd=source, env=env))
    packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
    with (args.out / "logs/build-baseline.log").open("wb") as log:
        subprocess.run(["cargo", "clean", "--release", *[v for p in packages for v in ["-p", p]]], cwd=source, env=env,
                       stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(["cargo", "build", "--release", "--locked", "--offline", "-p", "notist-cli", "--bin", "notist"], cwd=source, env=env,
                       stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(args.out / "target/release/notist", args.out / "bin/baseline")
    manifest["binaries"]["baseline"] = digest(args.out / "bin/baseline")
    save(args.out / "manifest.json", manifest)
    expected = json.loads((args.out / "output-inventory.json").read_text())
    rows = []
    for index in range(3):
        for kind in (["baseline", "normal"] if index % 2 == 0 else ["normal", "baseline"]):
            label = f"paired-{index}-{kind}"
            started = time.perf_counter()
            with (args.out / "logs" / f"{label}.stdout").open("wb") as stdout, (args.out / "logs" / f"{label}.stderr").open("wb") as stderr:
                child = subprocess.Popen([str(args.out / "bin" / kind), "build"], cwd=args.out / "site", env=environment(), stdout=stdout, stderr=stderr)
                _, status, usage = os.wait4(child.pid, 0)
                child.returncode = os.waitstatus_to_exitcode(status)
            row = dict(kind=kind, round=index, wall_s=time.perf_counter()-started,
                       user_s=usage.ru_utime, system_s=usage.ru_stime, maxrss_kib=usage.ru_maxrss,
                       exit=child.returncode, load=Path("/proc/loadavg").read_text().strip())
            save(args.out / "logs" / f"{label}.json", row)
            assert child.returncode == 0, label
            actual = inventory(args.out / "site/dist/icexiv-db")
            assert actual == expected, f"full HTML/asset output mismatch: {label}"
            row["complete_output_identical"] = True
            rows.append(row)
            save(args.out / "paired-samples.json", rows)
            print(label, row, flush=True)
    guard(args)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--baseline", default="e07b67e")
    args = parser.parse_args()
    args.out = args.out.resolve()
    run(args)
