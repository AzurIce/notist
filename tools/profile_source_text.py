#!/usr/bin/env python3
"""Shared source-text representation: frozen sources, field traffic and full A/B."""
import argparse
import json
import os
from pathlib import Path
import re
import shutil
import subprocess

import profile_boundaries as boundary
import profile_present_checkpoint as checkpoint
import profile_runtime_cow as common

FILES = [
    "crates/notist-core/src/item.rs",
    "crates/notist-pipeline/src/code/check.rs",
    "crates/notist-pipeline/src/code/data.rs",
    "crates/notist-pipeline/src/code/eval.rs",
    "crates/notist-pipeline/src/materialize.rs",
    "crates/notist-html/src/lib.rs",
    "crates/notist-book/src/lib.rs",
    "crates/notist-ssg/src/build.rs",
    "crates/notist-cli/src/query.rs",
    "src/json.rs",
    "src/preview.rs",
    "src/vault_index.rs",
]


def instrument(source):
    checkpoint.instrument(source)
    path = source / "crates/notist-core/src/boundary_probe.rs"
    path.write_text(boundary.once(path.read_text(), '"present_recovery"', '"expand"'))
    path = source / boundary.evaluator_path(source)
    text = path.read_text()
    operation = ("origin.expansion.extend_from_slice(locations)"
                 if "origin.expansion.extend_from_slice(locations)" in text
                 else "extended.extend_from_slice(locations)")
    text = boundary.once(text, operation, f"notist_core::boundary_probe::site(1, || notist_core::boundary_probe::field(4, || {operation}))")
    path.write_text(text)


def prepare(args):
    boundary.instrument = instrument
    boundary.prepare(args)
    for name in ["profile_source_text.py", "profile_present_checkpoint.py", "evaluator_ab_alloc.c"]:
        shutil.copy2(Path(__file__).with_name(name), args.out / "reproducer" / name)
    m = json.loads((args.out / "manifest.json").read_text())
    m["reproducer"] = common.inventory(args.out / "reproducer")
    common.save(args.out / "manifest.json", m)


def freeze_after(args):
    out = args.out
    m = json.loads((out / "manifest.json").read_text())
    after = out / "sources/after"
    assert not after.exists()
    shutil.copytree(out / "sources/before", after)
    repo = Path(__file__).resolve().parents[1]
    changed = {p for p, fingerprint in m["before_sources"].items()
               if p.endswith(".rs") and common.digest(repo / p) != fingerprint[1]}
    assert changed == set(FILES), changed
    for relative in FILES:
        shutil.copy2(repo / relative, after / relative)
    m["after_sources"] = common.inventory(after)
    m["source_text_files"] = FILES
    shutil.copy2(out / "reproducer/profile_runtime_cow.py", out / "reproducer.py")
    m["reproducer_sha256"] = common.digest(out / "reproducer.py")
    common.save(out / "manifest.json", m)


def heap(args):
    out = args.out
    assert not os.environ.get("LD_PRELOAD")
    assert not (out / "heap.json").exists()
    source = out / "reproducer/evaluator_ab_alloc.c"
    library = out / "source-text-alloc.so"
    subprocess.run(["cc", "-shared", "-fPIC", "-O2", str(source), "-o", str(library)], check=True)
    result = []
    for kind in ["before", "after"]:
        label = f"heap-{kind}"
        env = dict(os.environ, LD_PRELOAD=str(library), EVAL_AB_ALLOC_TARGET=str(out / "bin" / kind),
                   NO_COLOR="1", TERM="dumb", PYTHONDONTWRITEBYTECODE="1")
        with (out / "logs" / f"{label}.stdout").open("wb") as stdout, (out / "logs" / f"{label}.stderr").open("wb") as stderr:
            subprocess.run([str(out / "bin" / kind), "build"], cwd=out / "site", env=env,
                           stdout=stdout, stderr=stderr, check=True)
        text = (out / "logs" / f"{label}.stderr").read_text()
        records = re.findall(r"\[eval-native-alloc\] pid=(\d+) allocations=(\d+) requested_bytes=(\d+) frees=(\d+) reallocations=(\d+)", text)
        assert len(records) == 1
        assert common.inventory(out / "site/dist/icexiv-db") == json.loads((out / "output-inventory.json").read_text())
        result.append({"kind": kind, "output_verified": True, "api_events": dict(zip(
            ["pid", "allocations", "requested_bytes", "frees", "reallocations"], map(int, records[0])))})
        common.save(out / "heap.json", {"library_sha256": common.digest(library), "samples": result})
    print(json.dumps(result, indent=2), flush=True)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", choices=["prepare", "freeze_after", "probe", "probe_after", "heap"])
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--previous", type=Path, default=Path("/tmp/opencode/notist-present-checkpoint-20261104"))
    args = p.parse_args()
    args.out = args.out.resolve()
    boundary.instrument = instrument
    if args.action in ["prepare", "freeze_after", "heap"]:
        globals()[args.action](args)
    else:
        getattr(boundary, args.action)(args)


if __name__ == "__main__":
    main()
