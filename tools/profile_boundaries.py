#!/usr/bin/env python3
"""Field allocation attribution, then ordinary before/after recovery validation."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

import profile_runtime_cow as common

EVAL = "crates/notist-pipeline/src/code/eval.rs"
REPO = Path(__file__).resolve().parents[1]


def once(text, old, new):
    assert text.count(old) == 1, old
    return text.replace(old, new, 1)


def instrument(source):
    probe = "notist_core::boundary_probe"
    path = source / EVAL
    text = path.read_text()
    text = once(text, "let boundary = item_shell(&node);", f"let boundary = {probe}::site(1, || item_shell(&node));")
    if "let boundary = item_shell(&item);" in text:
        text = once(text, "let boundary = item_shell(&item);", f"let boundary = {probe}::site(2, || item_shell(&item));")
    else:
        assert "let mut boundary = Some(item);" in text
    text = once(text, "let mut captured = item_shell(item);", f"let mut captured = {probe}::site(3, || item_shell(item));")
    head, body = text.split("fn item_shell(item: &Item) -> Item {", 1)
    body, tail = body.split("fn failure_content", 1)
    for i, name in enumerate(["ctor", "instance", "target", "evaluation", "origin", "fields", "attrs"]):
        body = once(body, f"{name}: item.{name}.clone(),", f"{name}: {probe}::field({i}, || item.{name}.clone()),")
    path.write_text(head + "fn item_shell(item: &Item) -> Item {" + body + "fn failure_content" + tail)
    shutil.copy2(Path(__file__).with_name("boundary_probe.rs"), source / "crates/notist-core/src/boundary_probe.rs")
    path = source / "crates/notist-core/src/lib.rs"
    path.write_text(path.read_text() + "\n#[doc(hidden)]\npub mod boundary_probe;\n")
    path = source / "crates/notist-cli/src/main.rs"
    text = path.read_text()
    # CLI depends on the facade rather than directly on notist-core.
    text = once(text, "fn main() -> ExitCode {", "fn main() -> ExitCode {\n    let _boundary_report = notist::boundary_probe::Report;")
    path.write_text(text)
    path = source / "src/lib.rs"
    path.write_text(path.read_text() + "\n#[doc(hidden)]\npub use notist_core::boundary_probe;\n")


def prepare(args):
    out, old = args.out, args.previous
    assert not out.exists()
    m = json.loads((old / "manifest.json").read_text())
    before = old / "sources/after"
    assert common.inventory(before) == m["after_sources"]
    # Current production files must still be the accepted previous batch.
    for relative, fingerprint in m["after_sources"].items():
        if relative.endswith((".rs", "Cargo.toml", "Cargo.lock")):
            assert common.digest(REPO / relative) == fingerprint[1], relative
    for name in ["sources", "logs", "bin", "reproducer"]:
        (out / name).mkdir(parents=True, exist_ok=True)
    shutil.copytree(before, out / "sources/before")
    shutil.copytree(before, out / "sources/probe")
    instrument(out / "sources/probe")
    shutil.copytree(old / "site", out / "site", ignore=shutil.ignore_patterns("dist", "__pycache__"))
    assert common.inventory(out / "site") == m["inputs"]
    shutil.copy2(old / "output-inventory.json", out / "output-inventory.json")
    for name in ["profile_boundaries.py", "profile_runtime_cow.py", "boundary_probe.rs"]:
        shutil.copy2(REPO / "tools" / name, out / "reproducer" / name)
    manifest = {"before_sources": common.inventory(out / "sources/before"),
                "probe_sources": common.inventory(out / "sources/probe"), "inputs": m["inputs"],
                "binaries": {}, "test_modes": {}, "rounds": 6, "previous": str(old),
                "build_env": common.ENV, "reproducer": common.inventory(out / "reproducer")}
    common.save(out / "manifest.json", manifest)


def probe(args, kind="probe"):
    out = args.out
    m = json.loads((out / "manifest.json").read_text())
    source = out / "sources" / kind
    assert common.inventory(source) == m[kind + "_sources"]
    env = dict(os.environ, **common.ENV, CARGO_TARGET_DIR=str(out / "target"))
    with (out / "logs" / f"build-{kind}.log").open("wb") as log:
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=source, env=env))
        packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
        subprocess.run(["cargo", "clean", "--release", *[v for p in packages for v in ["-p", p]]], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist"], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(out / "target/release/notist", out / "bin" / kind)
    m["binaries"][kind] = common.digest(out / "bin" / kind)
    common.save(out / "manifest.json", m)
    label = "fields-before" if kind == "probe" else "fields-after"
    common.process(args, kind, label)
    records = {}
    for line in (out / "logs" / f"{label}.stderr").read_text().splitlines():
        if line.startswith("[boundary-field] "):
            _, site, field, calls, allocs, size, reallocs = line.split()
            records.setdefault(site, {})[field] = dict(zip(["calls", "allocations", "bytes", "reallocations"], map(int, [calls, allocs, size, reallocs])))
    assert len(records) == 4
    common.save(out / ("fields.json" if kind == "probe" else "fields-after.json"), records)
    print(json.dumps(records, indent=2), flush=True)


def probe_after(args):
    out = args.out
    m = json.loads((out / "manifest.json").read_text())
    source = out / "sources/after-probe"
    shutil.copytree(out / "sources/after", source)
    instrument(source)
    m["after-probe_sources"] = common.inventory(source)
    common.save(out / "manifest.json", m)
    probe(args, "after-probe")


def freeze_after(args):
    out = args.out
    m = json.loads((out / "manifest.json").read_text())
    assert not (out / "sources/after").exists()
    shutil.copytree(out / "sources/before", out / "sources/after")
    shutil.copy2(REPO / EVAL, out / "sources/after" / EVAL)
    m["after_sources"] = common.inventory(out / "sources/after")
    changed = {k for k in m["before_sources"] if m["before_sources"][k] != m["after_sources"][k]}
    assert changed == {EVAL}, changed
    m["reproducer_sha256"] = common.digest(out / "reproducer/profile_runtime_cow.py")
    # The shared validator verifies the exact helper it runs, not a live script.
    shutil.copy2(out / "reproducer/profile_runtime_cow.py", out / "reproducer.py")
    common.save(out / "manifest.json", m)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", choices=["prepare", "probe", "freeze_after", "probe_after"])
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--previous", type=Path, default=Path("/tmp/opencode/notist-runtime-cow-20261104"))
    args = p.parse_args()
    args.out = args.out.resolve()
    globals()[args.action](args)


if __name__ == "__main__":
    main()
