#!/usr/bin/env python3
"""Single-variable present recovery experiment using the established cohort."""
import argparse
import json
from pathlib import Path
import shutil

import profile_boundaries as boundary
import profile_runtime_cow as common

original_instrument = boundary.instrument


def instrument(source):
    original_instrument(source)
    path = source / "crates/notist-core/src/boundary_probe.rs"
    path.write_text(boundary.once(path.read_text(), '"call_recovery"', '"checkpoint"'))
    path = source / boundary.evaluator_path(source)
    text = path.read_text()
    if "fn before_call(" in text:
        head, body = text.split("fn before_call(", 1)
        body, tail = body.split("fn restore(", 1)
        for name, field in [("fields", 5), ("target", 2)]:
            body = boundary.once(body, f"item.{name}.clone()", f"notist_core::boundary_probe::site(2, || notist_core::boundary_probe::field({field}, || item.{name}.clone()))")
        path.write_text(head + "fn before_call(" + body + "fn restore(" + tail)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "probe", "freeze_after", "probe_after"])
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--previous", type=Path, default=Path("/tmp/opencode/notist-boundaries-20261104"))
    args = parser.parse_args()
    args.out = args.out.resolve()
    boundary.instrument = instrument
    getattr(boundary, args.action)(args)
    if args.action == "prepare":
        shutil.copy2(Path(__file__), args.out / "reproducer" / Path(__file__).name)
        manifest = json.loads((args.out / "manifest.json").read_text())
        manifest["reproducer"] = common.inventory(args.out / "reproducer")
        common.save(args.out / "manifest.json", manifest)


if __name__ == "__main__":
    main()
