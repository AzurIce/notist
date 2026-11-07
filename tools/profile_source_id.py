#!/usr/bin/env python3
"""Isolated 32-bit SourceId experiment on the frozen full kernel-profile site.

Build both versions with identical release flags. Interleave clean wall/RSS
runs; allocation diagnostics are separate, and every full output is checked
outside measurement. Never edits or publishes into the original consumer.
"""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time
import re

import profile_kernel_site as kernel
from profile_icexiv_refresh import digest, environment, freeze
from profile_xiv_history import save


def prepare(args):
    if args.out.exists() or args.out.is_relative_to(kernel.REPO):
        raise ValueError("use a fresh external directory")
    reference = kernel.guard(type("Args", (), {"out": args.reference})())
    for name in ["sources", "logs", "bin", "reproducer"]:
        (args.out / name).mkdir(parents=True, exist_ok=True)
    shutil.copytree(args.reference / "sources/normal", args.out / "sources/before")
    freeze(args.out / "sources/after")
    shutil.copytree(args.reference / "site", args.out / "site", symlinks=True,
                    ignore=shutil.ignore_patterns("dist", "__pycache__"))
    assert kernel.inventory(args.out / "site") == reference["inputs"]
    shutil.copy2(args.reference / "output-inventory.json", args.out / "output-inventory.json")
    for name in ["profile_source_id.py", "evaluator_ab_alloc.c"]:
        shutil.copy2(kernel.REPO / "tools" / name, args.out / "reproducer" / name)
    manifest = dict(reference=str(args.reference), before_revision=reference["revision"],
                    after_revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=kernel.REPO, text=True).strip(),
                    inputs=reference["inputs"], input_symlinks=reference["input_symlinks"],
                    build_env=kernel.ENV, rustc=subprocess.check_output(["rustc", "-Vv"], text=True),
                    sources={kind: kernel.inventory(args.out / "sources" / kind) for kind in ["before", "after"]},
                    binaries={}, reproducer=kernel.inventory(args.out / "reproducer"))
    (args.out / "workspace.patch").write_bytes(subprocess.check_output(["git", "diff", "--binary", "HEAD"], cwd=kernel.REPO))
    save(args.out / "manifest.json", manifest)
    print("prepared", args.out, flush=True)


def guard(args):
    manifest = json.loads((args.out / "manifest.json").read_text())
    assert kernel.inventory(args.out / "reproducer") == manifest["reproducer"]
    for kind, expected in manifest["sources"].items():
        assert kernel.inventory(args.out / "sources" / kind) == expected, kind
    for kind, expected in manifest["binaries"].items():
        assert digest(args.out / "bin" / kind) == expected, kind
    actual = kernel.inventory(args.out / "site")
    assert {k: v for k, v in actual.items() if not k.startswith("dist/")} == manifest["inputs"]
    links = {str(p.relative_to(args.out / "site")): str(p.readlink()) for p in (args.out / "site").rglob("*")
             if p.is_symlink() and "dist" not in p.relative_to(args.out / "site").parts}
    assert links == manifest["input_symlinks"]
    return manifest


def build(args):
    manifest = guard(args)
    env = dict(os.environ, **kernel.ENV, CARGO_TARGET_DIR=str(args.out / "target"))
    for kind in ["before", "after"]:
        source = args.out / "sources" / kind
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version=1"], cwd=source, env=env))
        packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
        with (args.out / "logs" / f"build-{kind}.log").open("wb") as log:
            subprocess.run(["cargo", "clean", "--release", *[v for p in packages for v in ["-p", p]]], cwd=source, env=env,
                           stdout=log, stderr=subprocess.STDOUT, check=True)
            subprocess.run(["cargo", "build", "--release", "--locked", "--offline", "-p", "notist-cli", "--bin", "notist"],
                           cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(args.out / "target/release/notist", args.out / "bin" / kind)
        manifest["binaries"][kind] = digest(args.out / "bin" / kind)
        save(args.out / "manifest.json", manifest)
        print("built", kind, flush=True)


def run_once(args, kind, label, preload=None):
    manifest = json.loads((args.out / "manifest.json").read_text())
    binary = args.out / "bin" / kind
    assert digest(binary) == manifest["binaries"][kind]
    env = environment()
    if preload:
        env.update(LD_PRELOAD=str(preload), EVAL_AB_ALLOC_TARGET=str(binary))
    elif env.get("LD_PRELOAD"):
        raise ValueError("clean run must not inherit a preload")
    started = time.perf_counter()
    with (args.out / "logs" / f"{label}.stdout").open("wb") as stdout, (args.out / "logs" / f"{label}.stderr").open("wb") as stderr:
        child = subprocess.Popen([str(binary), "build"], cwd=args.out / "site", env=env, stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    row = dict(kind=kind, label=label, wall_s=time.perf_counter()-started,
               user_s=usage.ru_utime, system_s=usage.ru_stime, maxrss_kib=usage.ru_maxrss,
               exit=child.returncode, diagnostic=bool(preload), load=Path("/proc/loadavg").read_text().strip())
    assert child.returncode == 0, label
    assert kernel.inventory(args.out / "site/dist/icexiv-db") == json.loads((args.out / "output-inventory.json").read_text()), label
    row["complete_output_identical"] = True
    if preload:
        lines = [s for s in (args.out / "logs" / f"{label}.stderr").read_text().splitlines() if s.startswith("[eval-native-alloc] ")]
        assert len(lines) == 1, (label, lines)
        row["allocation_api"] = {k: int(v) for k,v in re.findall(r"(\w+)=(\d+)", lines[0]) if k != "pid"}
    save(args.out / "logs" / f"{label}.json", row)
    print(label, row, flush=True)
    return row


def measure(args):
    guard(args)
    if (args.out / "samples.json").exists():
        raise ValueError("do not overwrite samples")
    rows = []
    # Initial publication is outside the interleaved clean cohort.
    run_once(args, "before", "initial-publication")
    for index in range(5):
        for kind in (["before", "after"] if index % 2 == 0 else ["after", "before"]):
            rows.append(run_once(args, kind, f"clean-{index}-{kind}"))
            save(args.out / "samples.json", rows)
    summary = {kind: {field: statistics.median(r[field] for r in rows if r["kind"] == kind)
                     for field in ["wall_s", "user_s", "system_s", "maxrss_kib"]} for kind in ["before", "after"]}
    summary["change_percent"] = {f: 100*(summary["after"][f]/summary["before"][f]-1) for f in summary["before"]}
    save(args.out / "summary.json", summary)
    guard(args)


def allocations(args):
    guard(args)
    library = args.out / "bin/allocation-observer.so"
    subprocess.run(["cc", "-O2", "-shared", "-fPIC", str(args.out / "reproducer/evaluator_ab_alloc.c"), "-o", str(library)], check=True)
    rows = []
    for index in range(2):
        for kind in (["before", "after"] if index == 0 else ["after", "before"]):
            rows.append(run_once(args, kind, f"alloc-{index}-{kind}", library))
            save(args.out / "allocation-samples.json", rows)
    guard(args)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "build", "measure", "allocations"])
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--reference", type=Path, default=Path("/tmp/opencode/notist-icexiv-kernel-20261107-final"))
    args = parser.parse_args()
    args.out = args.out.resolve()
    globals()[args.action](args)
