#!/usr/bin/env python3
"""Independent evaluator drill-down; leaves the primary timing batch untouched."""
import argparse
import difflib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess

from profile_current import entries, guard, process
from profile_resource_overhead import fingerprints
from profile_xiv_history import once, save, sha

WORKLOADS = ["literal", "shared", "private", "wide"]
PREFIX = "notist_core::current_profile"


def instrument_evaluator(root):
    relative = "crates/notist-pipeline/src/code/eval.rs"
    entries(root, relative, [("execute", "eval.execute"), ("implementation", "eval.implementation"),
            ("capture_expr", "eval.capture_expr"), ("capture_call", "eval.capture_call"),
            ("content_count", "eval.content_count"), ("valid_structure", "eval.valid_structure"),
            ("expand", "eval.expand_origin"), ("check_value_size", "eval.check_value_size")])
    path = root / relative
    text = path.read_text()
    # Time the existing operations, never skip cloning, quotas or validation.
    for original, label in [
        ("let mut item = item.clone();", "eval.clone_item"),
        ("let mut item = node.clone();", "eval.clone_item"),
        ("let mut expr = expr.clone();", "eval.clone_typed_expr"),
        ("let mut call = call.clone();", "eval.clone_invocation"),
    ]:
        assert original in text
        binding, expression = original[:-1].split(" = ", 1)
        text = text.replace(original, f'{binding} = {PREFIX}::measure("{label}", || {expression});')
    text = once(text, "            item.children.clone(),", f'            {PREFIX}::measure("eval.clone_children", || item.children.clone()),')
    text = once(text, "            Some(item.clone()),", f'            Some({PREFIX}::measure("eval.clone_prototype", || item.clone())),')
    text = once(text, "        Ok((record.definition.clone(), record.body.clone()))",
        f'        let definition = {PREFIX}::measure("eval.clone_definition", || record.definition.clone());\n'
        f'        let body = {PREFIX}::measure("eval.clone_body", || record.body.clone());\n'
        "        Ok((definition, body))")
    path.write_text(text)


def build(args):
    out = args.out.resolve()
    main = guard(args)
    source = out / "evaluator-sources"
    assert not source.exists(), "use a fresh supplementary snapshot"
    shutil.copytree(out / "sources/detail", source)
    instrument_evaluator(source)
    patches = []
    relative = Path("crates/notist-pipeline/src/code/eval.rs")
    patches.extend(difflib.unified_diff((out / "sources/detail" / relative).read_text().splitlines(True),
        (source / relative).read_text().splitlines(True), fromfile="a/" + str(relative), tofile="b/" + str(relative)))
    (out / "patches/evaluator-extra.patch").write_text("".join(patches))
    env = dict(os.environ, **main["build_env"], CARGO_TARGET_DIR=str(out / "target-evaluator"), NO_COLOR="1", TERM="dumb")
    with (out / "logs/build-evaluator.log").open("wb") as log:
        subprocess.run(["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist", "--features", "profiling"],
                       cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(out / "target-evaluator/release/notist", out / "bin/evaluator")
    save(out / "evaluator-manifest.json", {"revision": main["revision"], "source": fingerprints(source),
         "binary_sha256": sha((out / "bin/evaluator").read_bytes()), "baseline_sha256": main["binaries"]["detail"]})


def run(args):
    out = args.out.resolve()
    guard(args)
    m = json.loads((out / "evaluator-manifest.json").read_text())
    assert fingerprints(out / "evaluator-sources") == m["source"]
    assert sha((out / "bin/evaluator").read_bytes()) == m["binary_sha256"]
    samples = []
    for rep in range(-1, 3):
        for wi, workload in enumerate(WORKLOADS):
            modes = ["detail", "evaluator"] if (rep + wi) % 2 else ["evaluator", "detail"]
            for kind in modes:
                label = f"eval-{'warmup' if rep < 0 else str(rep)}-{workload}-{kind}"
                sample = process(args, "detail", workload, label, child_command=[str(out / "bin" / kind), "build"])
                sample.update(kind=kind, round=rep)
                samples.append(sample)
                save(out / "evaluator-samples.json", samples)
                print(label, round(sample["wall_s"], 4), flush=True)
    rows = []
    for workload in WORKLOADS:
        row = {"workload": workload}
        for kind in ["detail", "evaluator"]:
            selected = [s for s in samples if s["kind"] == kind and s["workload"] == workload and s["round"] >= 0]
            wall = [s["wall_s"] for s in selected]
            names = set().union(*(s["metrics"] for s in selected))
            metrics = {name: {field: statistics.mean(s["metrics"][name][field] for s in selected)
                       for field in selected[0]["metrics"][name]} for name in names}
            row[kind] = {"wall_mean": statistics.mean(wall), "wall_median": statistics.median(wall),
                         "metrics": metrics, "modules": selected[0]["modules"]}
        assert {k: v["calls"] for k, v in row["detail"]["modules"].items()} == {k: v["calls"] for k, v in row["evaluator"]["modules"].items()}
        rows.append(row)
    save(out / "evaluator-summary.json", rows)
    guard(args)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["build", "run"])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cpu", type=int, default=8)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--probes", type=int, default=3)
    args = parser.parse_args()
    {"build": build, "run": run}[args.action](args)
