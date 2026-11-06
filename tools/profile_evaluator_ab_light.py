#!/usr/bin/env python3
"""Independent lightweight stages; no evaluator/global-allocator observer."""
import argparse
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess

from profile_icexiv import instrument, inventory
from profile_evaluator_ab import ENV, patch_between, process
from profile_current import parse_detail
from profile_xiv_history import once, save, sha


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("out", type=Path)
    args = parser.parse_args()
    args.out = args.out.resolve()
    out = args.out
    assert not (out / "light.json").exists()
    diagnostics = json.loads((out / "diagnostics.json").read_text())
    schema = next(row for row in diagnostics if row["kind"] == "base-probe" and row.get("mode") == "off")
    proof = {"sources": {}, "binaries": {}, "samples": []}
    for kind in ["base-light", "combined-light"]:
        source = out / "sources" / kind
        original = out / "sources" / kind.removesuffix("-light")
        shutil.copytree(original, source)
        instrument(source, False)
        site = source / "crates/notist-ssg/src/build.rs"
        text = site.read_text()
        for name in ["values", "dirs", "components", "by_source", "engine", "documents", "handles"]:
            text = once(text, f'    notist::xiv_profile::measure("drop.{name}", || drop({name}));\n', "")
        site.write_text(text)
        sink = source / "crates/notist-core/src/current_profile.rs"
        code = '\npub fn reserve_probe() { STATE.with(|cell| { let mut state = cell.borrow_mut(); state.stack.reserve(64);\n'
        for name in schema["stages"]:
            code += f'    state.metrics.entry("{name}").or_default();\n'
        for edge in schema["edges"]:
            code += f'    state.edges.entry(("{edge["parent"]}", "{edge["name"]}")).or_default();\n'
        sink.write_text(sink.read_text() + code + '}); }\n')
        adapter = source / "src/xiv_profile.rs"
        adapter.write_text(adapter.read_text() + '\npub use notist_core::current_profile::reserve_probe;\n')
        main = source / "crates/notist-cli/src/main.rs"
        main.write_text(once(main.read_text(), 'let _report = notist::xiv_profile::Report;', 'notist::xiv_profile::reserve_probe();\nlet _report = notist::xiv_profile::Report;'))
        (out / "patches" / f"{kind}.patch").write_text(patch_between(original, source))
        env = dict(os.environ, **ENV, CARGO_TARGET_DIR=str(out / "target"))
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=source, env=env))
        packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
        with (out / "logs" / f"build-{kind}.log").open("wb") as log:
            subprocess.run(["cargo", "clean", "--release", *[flag for p in packages for flag in ["-p", p]]], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            subprocess.run(["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist", "--features", "profiling"], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(out / "target/release/notist", out / "bin" / kind)
        proof["sources"][kind] = inventory(source)
        proof["binaries"][kind] = sha((out / "bin" / kind).read_bytes())
        save(out / "light.json", proof)
        print("built", kind, flush=True)
    shutil.copy2(Path(__file__), out / "light-reproducer.py")
    proof["reproducer_sha256"] = sha((out / "light-reproducer.py").read_bytes())
    for rep in range(-1, 3):
        kinds = ["base-light", "combined-light"] if rep % 2 else ["combined-light", "base-light"]
        for kind in kinds:
            label = f"light-{rep}-{kind}"
            row = process(args, kind, label)
            row["stages"], row["edges"], row["modules"] = parse_detail((out / "logs" / f"{label}.stderr").read_text())
            row["round"] = rep
            proof["samples"].append(row)
            save(out / "light.json", proof)
    phases = ["ssg.discover", "ssg.extension", "ssg.source_directories", "ssg.pipeline", "ssg.assets", "ssg.bodies", "ssg.shared_context", "ssg.templates", "ssg.finish"]
    summary = {}
    for kind in ["base-light", "combined-light"]:
        rows = [row for row in proof["samples"] if row["kind"] == kind and row["round"] >= 0]
        metrics = {name: statistics.mean(row["stages"][name]["seconds"] for row in rows) for name in rows[0]["stages"]}
        metrics["ssg.teardown"] = metrics["ssg.total"] - sum(metrics[name] for name in phases)
        metrics["combined_teardown"] = metrics["ssg.teardown"] + metrics["drop.vault"]
        summary[kind] = {"stages_mean_s": metrics, "wall_s": [row["wall_s"] for row in rows]}
        assert inventory(out / "sources" / kind) == proof["sources"][kind]
        assert sha((out / "bin" / kind).read_bytes()) == proof["binaries"][kind]
    save(out / "light-summary.json", summary)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
