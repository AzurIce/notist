#!/usr/bin/env python3
"""Freeze and compare the complete SourceId refactor; never modify the consumer.

Usage: profile_source_identity.py {prepare,build,measure,allocations,phases,verify}
       --out /tmp/opencode/FRESH --reference /tmp/opencode/FROZEN_KERNEL
The reference supplies the frozen site and output inventory, not a candidate.
Baseline sources come from HEAD; candidate sources include untracked Rust files.
"""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import statistics
import subprocess
import tarfile
import time

from profile_icexiv_refresh import environment, freeze
from profile_icexiv_refresh import instrument
from profile_current import parse_detail
from profile_support import BUILD_ENV, digest, inventory, save

REPO = Path(__file__).resolve().parents[1]


def prepare(args):
    if args.out.exists() or args.out.is_relative_to(REPO):
        raise ValueError("use a fresh external directory")
    reference = json.loads((args.reference / "manifest.json").read_text())
    for name in ["sources/before", "logs", "bin"]:
        (args.out / name).mkdir(parents=True, exist_ok=True)
    archive = args.out / "baseline.tar"
    subprocess.run(["git", "archive", "-o", str(archive), "HEAD"], cwd=REPO, check=True)
    with tarfile.open(archive) as stream:
        stream.extractall(args.out / "sources/before", filter="data")
    freeze(args.out / "sources/after")
    shutil.copytree(args.reference / "site", args.out / "site", symlinks=True,
                    ignore=shutil.ignore_patterns("dist", "__pycache__"))
    assert inventory(args.out / "site") == reference["inputs"]
    shutil.copy2(args.reference / "output-inventory.json", args.out / "output-inventory.json")
    manifest = dict(
        baseline_revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
        build_env=BUILD_ENV,
        rustc=subprocess.check_output(["rustc", "-Vv"], text=True),
        inputs=reference["inputs"],
        input_symlinks={str(p.relative_to(args.out / "site")): str(p.readlink())
                        for p in (args.out / "site").rglob("*") if p.is_symlink()},
        sources={kind: inventory(args.out / "sources" / kind) for kind in ["before", "after"]},
        binaries={},
    )
    (args.out / "workspace.patch").write_bytes(subprocess.check_output(["git", "diff", "--binary", "HEAD"], cwd=REPO))
    save(args.out / "manifest.json", manifest)


def verify(args):
    manifest = json.loads((args.out / "manifest.json").read_text())
    for kind, expected in manifest["sources"].items():
        assert inventory(args.out / "sources" / kind) == expected, kind
    for kind, expected in manifest["binaries"].items():
        assert digest(args.out / "bin" / kind) == expected, kind
    actual = inventory(args.out / "site")
    assert {k: v for k, v in actual.items() if not k.startswith("dist/")} == manifest["inputs"]
    links = {str(p.relative_to(args.out / "site")): str(p.readlink())
             for p in (args.out / "site").rglob("*") if p.is_symlink() and "dist" not in p.relative_to(args.out / "site").parts}
    assert links == manifest["input_symlinks"]
    return manifest


def build(args):
    if any((args.out / name).exists() for name in ["samples.json", "allocation-samples.json"]):
        raise ValueError("do not rebuild binaries backing existing measurements")
    manifest = verify(args)
    env = dict(os.environ, **BUILD_ENV, CARGO_TARGET_DIR=str(args.out / "target"))
    for kind in ["before", "after"]:
        source = args.out / "sources" / kind
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version=1"], cwd=source, env=env))
        packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
        with (args.out / "logs" / f"build-{kind}.log").open("wb") as log:
            subprocess.run(["cargo", "clean", "--release", *[v for p in packages for v in ["-p", p]]], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            subprocess.run(["cargo", "build", "--release", "--locked", "--offline", "-p", "notist-cli", "--bin", "notist"], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
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
        raise ValueError("clean run must not inherit LD_PRELOAD")
    started = time.perf_counter()
    with (args.out / "logs" / f"{label}.stdout").open("wb") as stdout, (args.out / "logs" / f"{label}.stderr").open("wb") as stderr:
        child = subprocess.Popen([str(binary), "build"], cwd=args.out / "site", env=env, stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    row = dict(kind=kind, label=label, wall_s=time.perf_counter()-started,
               user_s=usage.ru_utime, system_s=usage.ru_stime, maxrss_kib=usage.ru_maxrss,
               exit=child.returncode, diagnostic=bool(preload) or kind.endswith("-phases"), load=Path("/proc/loadavg").read_text().strip())
    assert child.returncode == 0, label
    assert inventory(args.out / "site/dist/icexiv-db") == json.loads((args.out / "output-inventory.json").read_text()), label
    row["complete_output_identical"] = True
    if preload:
        lines = [s for s in (args.out / "logs" / f"{label}.stderr").read_text().splitlines() if s.startswith("[eval-native-alloc] ")]
        assert len(lines) == 1, (label, lines)
        row["allocation_api"] = {k: int(v) for k, v in re.findall(r"(\w+)=(\d+)", lines[0]) if k != "pid"}
    save(args.out / "logs" / f"{label}.json", row)
    print(label, row, flush=True)
    return row


def measure(args):
    verify(args)
    if (args.out / "samples.json").exists():
        raise ValueError("do not overwrite measurements")
    rows = []
    run_once(args, "before", "initial-publication")
    for index in range(5):
        for kind in (["before", "after"] if index % 2 == 0 else ["after", "before"]):
            rows.append(run_once(args, kind, f"clean-{index}-{kind}"))
            save(args.out / "samples.json", rows)
    summarize(args.out / "summary.json", rows, ["wall_s", "user_s", "system_s", "maxrss_kib"])
    verify(args)


def summarize(path, rows, fields):
    summary = {kind: {field: statistics.median(r[field] for r in rows if r["kind"] == kind)
                      for field in fields} for kind in ["before", "after"]}
    summary["change_percent"] = {f: 100*(summary["after"][f]/summary["before"][f]-1) for f in fields if summary["before"][f]}
    save(path, summary)


def allocations(args):
    verify(args)
    if (args.out / "allocation-samples.json").exists():
        raise ValueError("do not overwrite measurements")
    observer = args.out / "sources/after/tools/evaluator_ab_alloc.c"
    library = args.out / "bin/allocation-observer.so"
    subprocess.run(["cc", "-O2", "-shared", "-fPIC", str(observer), "-o", str(library)], check=True)
    rows = []
    for index in range(2):
        for kind in (["before", "after"] if index == 0 else ["after", "before"]):
            rows.append(run_once(args, kind, f"alloc-{index}-{kind}", library))
            save(args.out / "allocation-samples.json", rows)
    fields = list(rows[0]["allocation_api"])
    summarize(args.out / "allocation-summary.json", [dict(kind=r["kind"], **r["allocation_api"]) for r in rows], fields)
    verify(args)


def phases(args):
    """Independent coarse attribution; never substitute its wall time for clean."""
    manifest = verify(args)
    if (args.out / "phase-sources").exists():
        raise ValueError("do not overwrite instrumented evidence")
    env = dict(os.environ, **BUILD_ENV, CARGO_TARGET_DIR=str(args.out / "target"))
    phase_sources = {}
    for kind in ["before", "after"]:
        source = args.out / "phase-sources" / kind
        shutil.copytree(args.out / "sources" / kind, source)
        instrument(source, False)
        phase_sources[kind] = inventory(source)
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version=1"], cwd=source, env=env))
        packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
        with (args.out / "logs" / f"build-{kind}-phases.log").open("wb") as log:
            subprocess.run(["cargo", "clean", "--release", *[v for p in packages for v in ["-p", p]]], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            subprocess.run(["cargo", "build", "--release", "--locked", "--offline", "--features", "profiling", "-p", "notist-cli", "--bin", "notist"], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        binary = args.out / "bin" / f"{kind}-phases"
        shutil.copy2(args.out / "target/release/notist", binary)
        manifest["binaries"][f"{kind}-phases"] = digest(binary)
        save(args.out / "manifest.json", manifest)
    save(args.out / "phase-source-inventory.json", phase_sources)
    rows = []
    for index in range(2):
        for kind in (["before", "after"] if index == 0 else ["after", "before"]):
            label = f"phases-{index}-{kind}"
            row = run_once(args, f"{kind}-phases", label)
            row["kind"] = kind
            row["diagnostic"] = True
            row["phase_metrics"], row["phase_edges"], row["module_metrics"] = parse_detail((args.out / "logs" / f"{label}.stderr").read_text())
            rows.append(row)
            save(args.out / "phase-samples.json", rows)
    summary = {kind: {name: statistics.median(row["phase_metrics"][name]["seconds"] for row in rows if row["kind"] == kind)
                      for name in rows[0]["phase_metrics"]} for kind in ["before", "after"]}
    save(args.out / "phase-summary.json", summary)
    assert all(inventory(args.out / "phase-sources" / kind) == expected for kind, expected in phase_sources.items())
    verify(args)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "build", "measure", "allocations", "phases", "verify"])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--reference", type=Path, default=Path("/tmp/opencode/notist-icexiv-kernel-20261107-final"))
    args = parser.parse_args()
    args.out = args.out.resolve()
    globals()[args.action](args)
