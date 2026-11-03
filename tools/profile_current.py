#!/usr/bin/env python3
"""Detailed profiling of one committed revision, with untouched full workloads."""
import argparse
import difflib
import json
import os
from pathlib import Path
import platform
import pstats
import shutil
import statistics
import subprocess
import sys
import time

from profile_history_matrix import WORKLOADS, inventory
from profile_resource_overhead import fingerprints, parse_strace
from profile_xiv_history import REPO, extract, git, once, save, sha
from xiv_history_probes import MODULE_STATS, enter_function, instrument, measure_rhs, parse_stages

KINDS = ["normal", "coarse", "detail"]
PHASES = ["ssg.discover", "ssg.prepare", "ssg.source_directories", "ssg.pipeline",
          "ssg.assets_templates", "ssg.bodies", "ssg.shared_context", "ssg.page_templates", "ssg.finish"]
ADAPTER = '''pub use notist_core::current_profile::{measure, span};
pub struct Report;
impl Drop for Report {
    fn drop(&mut self) {
        notist_core::current_profile::report();
        __MODULE_STATS__
    }
}
'''


def edit(root, relative, transform):
    path = root / relative
    path.write_text(transform(path.read_text()))


def entries(root, relative, methods, prefix="notist_core::current_profile"):
    def transform(text):
        for name, label in methods:
            text = enter_function(text, name, f'        let _current = {prefix}::span("{label}");')
        return text
    edit(root, relative, transform)


def detailed_instrument(root):
    """Only the disposable copy changes. No resource or cache policies change."""
    instrument(root, "current")
    core = root / "crates/notist-core/src"
    shutil.copy2(Path(__file__).with_name("current_profile_probe.rs"), core / "current_profile.rs")
    with (core / "lib.rs").open("a") as file:
        file.write("\n#[doc(hidden)]\npub mod current_profile;\n")
    (root / "src/xiv_profile.rs").write_text(ADAPTER.replace("__MODULE_STATS__", MODULE_STATS))
    prefix = "notist_core::current_profile"
    entries(root, "src/resource_session.rs", [("source_bytes", "input.source"), ("load", "input.load"),
            ("data", "input.data"), ("register_source", "input.register_document")])
    edit(root, "src/resource_session.rs", lambda text: once(text,
        "        if let Some(cached) = self.state.borrow().files.get(&path) {\n            return cached.read.clone();\n        }",
        '        if let Some(cached) = self.state.borrow().files.get(&path) {\n            notist_core::current_profile::count("input.source_hit", 1);\n            return cached.read.clone();\n        }\n        notist_core::current_profile::count("input.source_miss", 1);'))
    edit(root, "src/resources.rs", lambda text: text.replace("impl ResourceReader for FsReader {\n    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead {",
        'impl ResourceReader for FsReader {\n    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead {\n        let _current = notist_core::current_profile::span("io.bounded_read");', 1))
    # Top-level module loading and assembly, including config parsing per page.
    entries(root, "src/environment/loader.rs", [("document", "environment.document"), ("visit", "environment.visit"),
            ("resolve_imports", "environment.resolve_imports"), ("new", "environment.new_graph")])
    entries(root, "src/environment/config.rs", [("parse_config", "environment.parse_config")])
    # Built-in frontend parsing/lowering, without adding dependencies to syntax.
    for relative, methods in [
        ("frontend/markup/mod.rs", [("compile", "frontend.notist"), ("lower_content", "frontend.notist_content")]),
        ("frontend/markdown/mod.rs", [("parse_body", "frontend.markdown_parse"), ("lower_parsed", "frontend.markdown_lower")]),
        ("code/check.rs", [("document", "check.document"), ("install", "check.install")]),
        ("code/content.rs", [("content", "check.content")]),
        ("shape/mod.rs", [("shape", "check.shape"), ("shape_inline", "check.shape_inline")]),
        ("materialize.rs", [("materialize", "check.materialize")]),
        ("code/eval.rs", [("run", "eval.run"), ("constant", "eval.comptime")]),
        ("definitions.rs", [("compile_uncached", "module.compile_actual")]),
        ("modules.rs", [("bind_imports", "module.bind"), ("bind_cached", "module.bind_actual")]),
        ("code/data.rs", [("data_native", "data.native"), ("read_data", "data.read")]),
    ]:
        entries(root, "crates/notist-pipeline/src/" + relative, methods)
    edit(root, "crates/notist-pipeline/src/frontend/markup/mod.rs", lambda text: once(once(text,
        "let parsed = markup::parse_document(source);",
        f'let parsed = {prefix}::measure("frontend.notist_parse", || markup::parse_document(source));'),
        "let (forest, module_attrs, annotations) = lower_document(&parsed, &mut diagnostics);",
        f'let (forest, module_attrs, annotations) = {prefix}::measure("frontend.notist_lower", || lower_document(&parsed, &mut diagnostics));'))
    edit(root, "crates/notist-pipeline/src/code/content.rs", lambda text: once(text,
        "let r: Vec<_> = forest.iter().map(|e| self.prepare(e)).collect();",
        f'let r: Vec<_> = {prefix}::measure("check.resolve", || forest.iter().map(|e| self.prepare(e)).collect());'))
    edit(root, "crates/notist-pipeline/src/code/eval.rs", lambda text: once(once(text,
        "let forest = e.bind_content(\n        std::slice::from_ref(root),\n        &Scope::new(),\n        limits.content_nodes,\n    );",
        f'let forest = {prefix}::measure("eval.bind_content", || e.bind_content(\n        std::slice::from_ref(root),\n        &Scope::new(),\n        limits.content_nodes,\n    ));'),
        "let mut roots = e.present(&forest, limits.content_nodes);",
        f'let mut roots = {prefix}::measure("eval.present", || e.present(&forest, limits.content_nodes));'))
    entries(root, "crates/notist-core/src/data/parse.rs", [("parse", "data.parse_actual")], "crate::current_profile")
    entries(root, "crates/notist-core/src/data/mod.rs", [("decode", "data.decode_actual")], "crate::current_profile")
    edit(root, "crates/notist-core/src/data/cache.rs", lambda text: once(text,
        "        if let Some((_, result)) = state.parsed.get(&key) {",
        '        if let Some((_, result)) = state.parsed.get(&key) {\n            crate::current_profile::count("data.parse_hit", 1);'))
    entries(root, "crates/notist-ssg/src/assets.rs", [("data", "ssg.site_data"), ("prepare", "ssg.asset_prepare")], "notist::xiv_profile")
    entries(root, "crates/notist-ssg/src/templates.rs", [("load", "ssg.template_load")], "notist::xiv_profile")
    # Attribute large teardown costs explicitly on successful builds. The values
    # are dropped at the return boundary, in their original relative drop order.
    def site(text):
        text = enter_function(text, "new", '        let _current = notist::xiv_profile::span("template.shared_total");')
        text = measure_rhs(text, "let pages: Vec<_> = ", ";\n        let directories:", "template.shared_pages")
        text = measure_rhs(text, "let directories: BTreeMap<_, _> = ", ";\n        let site = ", "template.shared_directories")
        text = once(text, "Value::try_from_serializable(data).map_err", 'notist::xiv_profile::measure("template.shared_data", || Value::try_from_serializable(data)).map_err')
        text = once(text, "    drop(data);", '    notist::xiv_profile::measure("drop.site_json", || drop(data));')
        drops = "\n".join(f'    notist::xiv_profile::measure("drop.{label}", || drop({name}));' for name, label in [
            ("values", "template_values"), ("dirs", "directory_maps"), ("components", "component_map"),
            ("by_source", "source_index"), ("engine", "template_engine"), ("documents", "document_trees")])
        return once(text, "    drop(xiv_phase);\n    xiv_result\n}", "    drop(xiv_phase);\n" + drops + "\n    xiv_result\n}")
    edit(root, "crates/notist-ssg/src/build.rs", site)
    edit(root, "crates/notist-cli/src/ssg.rs", lambda text: once(once(text,
        "    result\n        .map(|artifact|", "    let current_result = result\n        .map(|artifact|"),
        "        .map_err(|e| (e, observed))\n}",
        '        .map_err(|e| (e, observed));\n    notist::xiv_profile::measure("drop.vault", || drop(vault));\n    current_result\n}'))
    edit(root, "crates/notist-cli/src/publication.rs", lambda text: once(text,
        "    Ok(())\n}", '    drop(retired);\n    notist::xiv_profile::measure("publish.cleanup_retired", || drop(backup));\n    notist::xiv_profile::measure("publish.cleanup_staging", || drop(staging));\n    Ok(())\n}'))


def prepare(args):
    out = args.out.resolve()
    if out.exists() or out == REPO or REPO in out.parents:
        raise ValueError("use a new result directory outside the checkout")
    revision = git(REPO, "rev-parse", args.revision).decode().strip()
    assert revision == git(REPO, "rev-parse", "origin/main").decode().strip(), "profile the pushed revision"
    old = json.loads((args.inputs / "manifest.json").read_text())
    assert fingerprints(args.inputs / "sites", exclude_dist=True) == old["inputs"]
    out.mkdir(parents=True)
    for name in ["bin", "logs", "inventories", "patches"]:
        (out / name).mkdir()
    m = {"revision": revision, "cpu": args.cpu, "rounds": {"normal": args.runs, "coarse": args.probes, "detail": args.probes},
         "rustc": subprocess.check_output(["rustc", "-Vv"], text=True), "system": platform.platform(),
         "build_env": {"CARGO_PROFILE_RELEASE_DEBUG": "0", "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "2"},
         "inputs_from": str(args.inputs.resolve()), "sources": {}, "binaries": {},
         "rss": "not collected; fork-inherited launcher high-water marks are not Rust-only peaks"}
    for kind in KINDS:
        source = out / "sources" / kind
        if kind == "normal":
            extract(REPO, revision, source)
        else:
            original = out / "sources/normal"
            shutil.copytree(original, source)
            if kind == "coarse":
                instrument(source, "current")
            else:
                detailed_instrument(source)
            patch = []
            for path in sorted(source.rglob("*.rs")):
                rel = path.relative_to(source)
                before = (original / rel).read_text() if (original / rel).exists() else ""
                after = path.read_text()
                if before != after:
                    patch.extend(difflib.unified_diff(before.splitlines(True), after.splitlines(True), fromfile="a/" + str(rel), tofile="b/" + str(rel)))
            (out / "patches" / f"{kind}.patch").write_text("".join(patch))
        m["sources"][kind] = fingerprints(source)
    for workload, _, _ in WORKLOADS:
        shutil.copytree(args.inputs / "sites" / workload, out / "sites" / workload, ignore=shutil.ignore_patterns("dist", "__pycache__"))
        shutil.copy2(args.inputs / "inventories" / f"{workload}-expected.json", out / "inventories" / f"{workload}-expected.json")
    # Reuse the genuine structured-data positive control, outside main rankings.
    control = Path("/tmp/opencode/notist-resource-profile-20261103/sites/data-smoke")
    shutil.copytree(control, out / "sites/data-smoke", ignore=shutil.ignore_patterns("dist", "__pycache__"))
    m["inputs"] = fingerprints(out / "sites", exclude_dist=True)
    (out / "reproducer").mkdir()
    for name in ["profile_current.py", "current_profile_probe.rs", "test_profile_current.py", "xiv_history_probes.py", "profile_history_matrix.py", "profile_resource_overhead.py", "profile_xiv_history.py", "profile_module_site.py"]:
        shutil.copy2(REPO / "tools" / name, out / "reproducer" / name)
    m["reproducer"] = fingerprints(out / "reproducer")
    save(out / "manifest.json", m)
    print("prepared", revision, flush=True)


def guard(args, binaries=True):
    out = args.out.resolve()
    m = json.loads((out / "manifest.json").read_text())
    assert args.cpu == m["cpu"] and args.runs == m["rounds"]["normal"] and args.probes == m["rounds"]["coarse"]
    assert fingerprints(out / "sites", exclude_dist=True) == m["inputs"], "inputs changed"
    for kind in KINDS:
        assert fingerprints(out / "sources" / kind) == m["sources"][kind], kind
        if binaries:
            assert sha((out / "bin" / kind).read_bytes()) == m["binaries"][kind]
    return m


def build(args):
    out = args.out.resolve()
    m = guard(args, binaries=False)
    for kind in KINDS:
        binary = out / "bin" / kind
        if binary.exists():
            assert sha(binary.read_bytes()) == m["binaries"][kind]
            continue
        env = dict(os.environ, **m["build_env"], CARGO_TARGET_DIR=str(out / f"target-{kind}"), NO_COLOR="1", TERM="dumb")
        command = ["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist"]
        if kind != "normal":
            command += ["--features", "profiling"]
        with (out / "logs" / f"build-{kind}.log").open("wb") as log:
            subprocess.run(command, cwd=out / "sources" / kind, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(out / f"target-{kind}/release/notist", binary)
        m["binaries"][kind] = sha(binary.read_bytes())
        save(out / "manifest.json", m)
        print("built", kind, flush=True)


def parse_detail(text):
    metrics, edges, modules = {}, [], {}
    for line in text.splitlines():
        fields = line.split()
        if line.startswith("[current-time] "):
            _, name, calls, inclusive, own, maximum = fields
            metrics[name] = {"calls": int(calls), "seconds": float(inclusive), "self_seconds": float(own), "max_seconds": float(maximum)}
        elif line.startswith("[current-edge] "):
            _, parent, name, calls, inclusive, own = fields
            edges.append({"parent": parent, "name": name, "calls": int(calls), "seconds": float(inclusive), "self_seconds": float(own)})
        elif line.startswith("[xiv-module] "):
            _, name, calls, seconds = fields
            modules[name] = {"calls": int(calls), "seconds": float(seconds)}
    assert metrics and modules
    root = metrics["cli.total"]["seconds"]
    assert abs(sum(m["self_seconds"] for m in metrics.values()) - root) < 0.00001
    return metrics, edges, modules


def process(args, kind, workload, label, extra_env=None, child_command=None):
    out = args.out.resolve()
    env = dict(os.environ, NO_COLOR="1", TERM="dumb", PYTHONDONTWRITEBYTECODE="1", **(extra_env or {}))
    for name in ["RESOURCE_PROFILE", "RESOURCE_DIAGNOSTIC", "XIV_PROFILE_REQUESTS"]:
        env.pop(name, None)
    command = ["taskset", "-c", str(args.cpu), *(child_command or [str(out / "bin" / kind), "build"])]
    started = time.perf_counter()
    with (out / "logs" / f"{label}.stdout").open("wb") as stdout, (out / "logs" / f"{label}.stderr").open("wb") as stderr:
        child = subprocess.Popen(command, cwd=out / "sites" / workload / "site", env=env, stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    sample = {"kind": kind, "workload": workload, "label": label, "command": command,
              "wall_s": time.perf_counter() - started, "user_s": usage.ru_utime, "system_s": usage.ru_stime, "exit": child.returncode}
    assert child.returncode == 0, sample
    if kind == "coarse":
        sample["metrics"], sample["modules"] = parse_stages(out / "logs" / f"{label}.stderr")
    elif kind == "detail":
        sample["metrics"], sample["edges"], sample["modules"] = parse_detail((out / "logs" / f"{label}.stderr").read_text())
    root = out / "sites" / workload / "site/dist" / ("xiv-db" if workload == "xiv" else "bench")
    actual = inventory(root)
    if workload == "data-smoke":
        assert (root / "index.html").read_text().count("记录：共享数据视图") == 2
        assert "private_note" not in (root / "index.html").read_text()
        expected = out / "inventories/data-smoke-expected.json"
        if not expected.exists():
            assert kind == "normal"
            save(expected, actual)
    else:
        expected = out / "inventories" / f"{workload}-expected.json"
    assert actual == json.loads(expected.read_text()), label
    save(out / "inventories" / f"{label}.json", actual)
    save(out / "logs" / f"{label}.json", sample)
    return sample


def run(args):
    out = args.out.resolve()
    m = guard(args)
    samples = []
    for rep in range(-1, max(m["rounds"].values())):
        cells = [(kind, w) for w, _, _ in WORKLOADS for kind in KINDS if rep < m["rounds"][kind]]
        shift = max(rep, 0) * 4 % len(cells)
        cells = cells[shift:] + cells[:shift]
        if rep % 2:
            cells.reverse()
        for kind, workload in cells:
            label = f"{'warmup' if rep < 0 else 'round-' + str(rep)}-{workload}-{kind}"
            sample = process(args, kind, workload, label)
            sample["round"] = rep
            samples.append(sample)
            save(out / "samples.json", samples)
            print(label, round(sample["wall_s"], 4), flush=True)
    for kind in KINDS:
        sample = process(args, kind, "data-smoke", f"control-data-{kind}")
        samples.append(sample)
        if kind == "detail":
            metrics = sample["metrics"]
            assert metrics["data.read"]["calls"] == 4 and metrics["data.parse_actual"]["calls"] == 2
            assert metrics["data.parse_hit"]["calls"] == 2
    save(out / "samples.json", samples)
    for workload, _, _ in WORKLOADS:
        rows = [s for s in samples if s["workload"] == workload and s["kind"] != "normal"]
        work = [{k: v["calls"] for k, v in s["modules"].items()} for s in rows]
        assert all(w == work[0] for w in work)
        assert all(s["metrics"].get("data.read", {"calls": 0})["calls"] == 0 for s in rows if s["kind"] == "detail")
    guard(args)


def python_profile(args):
    out = args.out.resolve()
    guard(args)
    wrapper = out / "python-wrapper"
    wrapper.mkdir()
    executable = wrapper / "python3"
    executable.write_text(f'#!/bin/sh\nexec "{sys.executable}" -m cProfile -o "{out / "python.pstats"}" "$@"\n')
    executable.chmod(0o755)
    sample = process(args, "coarse", "xiv", "python-cprofile-xiv", {"PATH": str(wrapper) + os.pathsep + os.environ["PATH"]})
    stats = pstats.Stats(str(out / "python.pstats"))
    rows = [{"file": file, "line": line, "function": name, "primitive_calls": values[0], "calls": values[1],
             "self_s": values[2], "cumulative_s": values[3]} for (file, line, name), values in stats.stats.items()]
    save(out / "python-profile.json", {"sample": sample, "total_s": stats.total_tt, "calls": stats.total_calls,
                                      "functions": sorted(rows, key=lambda r: r["self_s"], reverse=True)})
    guard(args)


def trace(args):
    out = args.out.resolve()
    guard(args)
    rows = []
    for workload in ["xiv", "private"]:
        log = out / "logs" / f"strace-{workload}.txt"
        command = ["strace", "-qq", "-f", "-c", "-e", "trace=%file,close,read,write,getdents64", "-o", str(log),
                   str(out / "bin/normal"), "build"]
        sample = process(args, "normal", workload, f"strace-{workload}", child_command=command)
        sample["syscalls"] = parse_strace(log.read_text())
        rows.append(sample)
    save(out / "strace.json", rows)
    guard(args)


def distribution(values):
    return {"median": statistics.median(values), "mean": statistics.mean(values), "min": min(values), "max": max(values), "samples": values}


def partitions(metrics):
    t = lambda name: metrics[name]["seconds"]
    row = {
        "prepare": t("ssg.discover") + t("ssg.prepare") + t("ssg.source_directories"),
        "analyze": t("pipeline.analyze"), "evaluate": t("pipeline.evaluate"),
        "pipeline_other": t("ssg.pipeline") - t("pipeline.analyze") - t("pipeline.evaluate"),
        "assets_templates": t("ssg.assets_templates"), "html_links": t("ssg.bodies"),
        "template_shared": t("ssg.shared_context"), "template_pages": t("ssg.page_templates"),
        "site_teardown": t("ssg.total") - sum(t(k) for k in PHASES),
        "publish": t("cli.publish"),
        "host_other": t("cli.total") - t("ssg.total") - t("cli.publish") + t("ssg.finish"),
    }
    assert all(v >= -1e-7 for v in row.values()), row
    assert abs(sum(row.values()) - t("cli.total")) < 1e-7
    return row


def report(args):
    out = args.out.resolve()
    m = guard(args)
    samples = json.loads((out / "samples.json").read_text())
    rows = []
    for workload, title, pages in WORKLOADS:
        row = {"workload": workload, "title": title, "html": pages}
        for kind in KINDS:
            selected = [s for s in samples if s["kind"] == kind and s["workload"] == workload and s.get("round", -1) >= 0]
            row[kind] = {"wall_s": distribution([s["wall_s"] for s in selected]),
                         "user_s": distribution([s["user_s"] for s in selected]), "system_s": distribution([s["system_s"] for s in selected])}
            if kind != "normal":
                names = sorted(set().union(*(s["metrics"] for s in selected)))
                row[kind]["metrics"] = {name: {field: statistics.mean(s["metrics"].get(name, {}).get(field, 0) for s in selected)
                    for field in selected[0]["metrics"].get(name, {"calls": 0, "seconds": 0})} for name in names}
                row[kind]["modules"] = selected[0]["modules"]
            if kind == "coarse":
                split = [partitions(s["metrics"]) for s in selected]
                row["partitions_mean_s"] = {name: statistics.mean(s[name] for s in split) for name in split[0]}
            if kind == "detail":
                row["top_self"] = sorted(({"name": k, **v} for k, v in row[kind]["metrics"].items() if v.get("self_seconds", 0) > 0), key=lambda r: r["self_seconds"], reverse=True)
        rows.append(row)
    save(out / "summary.json", {"revision": m["revision"], "rows": rows})
    verified = list((out / "inventories").glob("*.json"))
    extra = int((out / "python-profile.json").exists())
    extra += len(json.loads((out / "strace.json").read_text())) if (out / "strace.json").exists() else 0
    save(out / "integrity.json", {"revision": m["revision"], "builds": len(samples) + extra,
         "timed_normal_builds": args.runs * len(WORKLOADS), "main_output_checks": sum(s["workload"] != "data-smoke" for s in samples),
         "inventories": len(verified), "source_binary_and_input_fingerprints_match": True,
         "binaries": m["binaries"], "probe_overhead_not_used_as_normal_timing": True})
    print(json.dumps({r["workload"]: r["normal"]["wall_s"]["median"] for r in rows}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "build", "run", "python", "trace", "report"])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--inputs", type=Path, default=Path("/tmp/opencode/notist-input-batch-20261103"))
    parser.add_argument("--revision", default="HEAD")
    parser.add_argument("--cpu", type=int, default=8)
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--probes", type=int, default=3)
    args = parser.parse_args()
    {"prepare": prepare, "build": build, "run": run, "python": python_profile, "trace": trace, "report": report}[args.action](args)


if __name__ == "__main__":
    main()
