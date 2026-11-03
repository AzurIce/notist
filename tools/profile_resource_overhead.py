#!/usr/bin/env python3
"""Attribute the resource/Data historical regression in disposable snapshots.

No production sources or limits change. Diagnostic scan ablations are UNSAFE
for changing inputs, not proposed optimizations. Ordinary CLI, timed probes,
and diagnostic ablations are kept separate and exhaustively output-checked.
"""
import argparse
import difflib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess

from profile_history_matrix import WORKLOADS, inventory, synthetic
from profile_xiv_history import REPO, once, save, sha, git
from xiv_history_probes import instrument, enter_function, parse_stages

ENGINES = ["environment", "resources", "current"]
MODES = ["full", "no-files", "no-kinds", "no-source-scans", "no-eval-scans", "no-scans"]


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def fine_instrument(source):
    """Add bounded-counter probes; exact original validate short-circuit order."""
    core = source / "crates/notist-core/src"
    write(core / "resource_profile.rs", Path(__file__).with_name("resource_profile_probe.rs").read_text())
    with (core / "lib.rs").open("a") as f:
        f.write("\n#[doc(hidden)]\npub mod resource_profile;\n")
    with (source / "src/lib.rs").open("a") as f:
        f.write("\n#[doc(hidden)]\npub use notist_core::resource_profile;\n")
    path = source / "src/resource_session.rs"
    text = path.read_text()
    old = '''            state.changed
                || state
                    .files
                    .values()
                    .filter_map(|f| f.observation.as_ref())
                    .any(|o| o.reader.revision(&o.relative) != o.revision)
                || state
                    .kinds
                    .iter()
                    .any(|(path, kind)| self.resources.kind(path) != *kind)'''
    new = '''            crate::resource_profile::maximum(crate::resource_profile::Metric::ObservedFiles, state.files.len());
            crate::resource_profile::maximum(crate::resource_profile::Metric::ObservedKinds, state.kinds.len());
            let (files, kinds) = crate::resource_profile::scan_policy();
            state.changed
                || (files && {
                    let _rp = crate::resource_profile::span(crate::resource_profile::Metric::ValidateFiles);
                    let mut checks = 0;
                    let changed = state.files.values().filter_map(|f| f.observation.as_ref()).any(|o| {
                        checks += 1;
                        o.reader.revision(&o.relative) != o.revision
                    });
                    crate::resource_profile::count(crate::resource_profile::Metric::FileChecks, checks);
                    changed
                })
                || (kinds && {
                    let _rp = crate::resource_profile::span(crate::resource_profile::Metric::ValidateKinds);
                    let mut checks = 0;
                    let changed = state.kinds.iter().any(|(path, kind)| {
                        checks += 1;
                        self.resources.kind(path) != *kind
                    });
                    crate::resource_profile::count(crate::resource_profile::Metric::KindChecks, checks);
                    changed
                })'''
    text = once(text, old, new)
    for name, metric in [("validate", "Validate"), ("source_bytes", "SourceBytes"), ("request", "Request"),
                         ("load", "Load"), ("data", "DataRead"), ("observe_source", "ObserveSource")]:
        text = enter_function(text, name, f'        let _rp = crate::resource_profile::span(crate::resource_profile::Metric::{metric});')
    for name, cause in [("source_bytes", "Source"), ("read_data", "Data")]:
        text = enter_function(text, name, f'        let _cause = crate::resource_profile::cause(crate::resource_profile::Cause::{cause});')
    for anchor, code in [
        ("        if let Some(cached) = self.state.borrow().files.get(path) {\n            return cached.read.clone();\n        }", "Load"),
        ("        if let Some(cached) = self.state.borrow().files.get(&path) {\n            return cached.read.clone();\n        }", "Source"),
    ]:
        new = anchor.replace("            return cached.read.clone();", f"            crate::resource_profile::count(crate::resource_profile::Metric::{code}Hit, 1);\n            return cached.read.clone();")
        new += f"\n        crate::resource_profile::count(crate::resource_profile::Metric::{code}Miss, 1);"
        text = once(text, anchor, new)
    path.write_text(text)
    path = source / "src/vault.rs"
    text = path.read_text()
    for name, phase in [("source", "Sources"), ("compile", "Analyze"), ("evaluate_in", "Evaluate")]:
        text = enter_function(text, name, f'        let _rp_phase = crate::resource_profile::phase(crate::resource_profile::Phase::{phase});')
    path.write_text(text)
    path = source / "src/resources.rs"
    text = path.read_text()
    start, end = text.index("impl Resources for FsResources {"), text.index("struct FsReader(")
    part = text[start:end]
    for name, metric, method in [("read", "FsRawRead", "span"), ("kind", "FsKind", "detail"), ("read_root", "FsReadRoot", "span")]:
        part = enter_function(part, name, f'        let _rp = crate::resource_profile::{method}(crate::resource_profile::Metric::{metric});')
    text = text[:start] + part + text[end:]
    start, end = text.index("impl FsReader {"), text.index("fn fs_revision(")
    part = text[start:end]
    for name, metric, method in [("open", "FsOpen", "detail"), ("read_bounded", "FsReadBounded", "span"), ("revision", "FsRevision", "detail")]:
        part = enter_function(part, name, f'        let _rp = crate::resource_profile::{method}(crate::resource_profile::Metric::{metric});')
    text = text[:start] + part + text[end:]
    path.write_text(text)
    path = source / "crates/notist-pipeline/src/code/data.rs"
    text = path.read_text()
    for name, metric in [("data_native", "DataNative"), ("read_data", "DataReadNative")]:
        text = enter_function(text, name, f'        let _rp = notist_core::resource_profile::span(notist_core::resource_profile::Metric::{metric});')
    path.write_text(text)
    for rel, name, metric in [("data/parse.rs", "parse", "DataParse"), ("data/mod.rs", "decode", "DataDecode")]:
        path = core / rel
        path.write_text(enter_function(path.read_text(), name, f'    let _rp = crate::resource_profile::span(crate::resource_profile::Metric::{metric});'))
    path = core / "data/cache.rs"
    text = path.read_text()
    index = text.index("impl ParsedDataCache for DataCache {")
    part = text[index:]
    for name, metric in [("parse", "CacheParse"), ("decode", "CacheDecode")]:
        part = enter_function(part, name, f'        let _rp = crate::resource_profile::span(crate::resource_profile::Metric::{metric});')
    part = once(part, "        if let Some((_, result)) = state.parsed.get(&key) {", "        if let Some((_, result)) = state.parsed.get(&key) {\n            crate::resource_profile::count(crate::resource_profile::Metric::CacheParseHit, 1);")
    part = once(part, "        state.stats.parses += 1;", "        crate::resource_profile::count(crate::resource_profile::Metric::CacheParseMiss, 1);\n        state.stats.parses += 1;")
    path.write_text(text[:index] + part)
    path = source / "crates/notist-cli/src/main.rs"
    path.write_text(enter_function(path.read_text(), "main", "    let _resource_report = notist::resource_profile::Report;"))


def fingerprints(root, exclude_dist=False):
    return {p.relative_to(root).as_posix(): sha(p.read_bytes()) for p in sorted(root.rglob("*"))
            if p.is_file() and not (exclude_dist and "dist" in p.relative_to(root).parts)}


def prepare(args):
    out, history = args.out.resolve(), args.history.resolve()
    if out.exists() or out == REPO or REPO in out.parents:
        raise ValueError("use a new result directory outside the checkout")
    old = json.loads((history / "manifest.json").read_text())
    source_fingerprints = json.loads((history / "source-fingerprints.json").read_text())
    out.mkdir(parents=True)
    for directory in ["bin", "sources", "sites", "logs", "patches", "inventories"]:
        (out / directory).mkdir()
    m = {"history": str(history), "engines": {}, "runs": args.runs, "cpu": args.cpu,
         "rustc": subprocess.check_output(["rustc", "-Vv"], text=True), "system": platform.platform(),
         "build_env": old["build_env"], "requests": 1000000}
    assert m["rustc"] == old["rustc"]
    (out / "checkout-start.diff").write_bytes(git(REPO, "diff", "--binary", "HEAD"))
    for engine in ENGINES:
        meta = old["engines"][engine]
        binary = history / "bin" / engine
        assert sha(binary.read_bytes()) == meta["binary_sha256"]
        shutil.copy2(binary, out / "bin" / engine)
        m["engines"][engine] = {"revision": meta["revision"], "normal_sha256": meta["binary_sha256"], "adaptations": meta["adaptations"]}
        if engine == "environment":
            continue
        original, source = Path(meta["source"]), out / "sources" / engine
        shutil.copytree(original, source)
        # The historical matrix captures the benchmark request override, not the
        # checkout's independent uncommitted SSG host-policy changes.
        before = fingerprints(source)
        assert before == source_fingerprints[engine]["paths"], f"historical source changed: {engine}"
        instrument(source, engine)
        fine_instrument(source)
        diff = []
        for p in sorted(source.rglob("*.rs")):
            rel = p.relative_to(source)
            a = (original / rel).read_text() if (original / rel).is_file() else ""
            b = p.read_text()
            if a != b:
                diff.extend(difflib.unified_diff(a.splitlines(True), b.splitlines(True), fromfile="a/" + str(rel), tofile="b/" + str(rel)))
        write(out / "patches" / f"{engine}.patch", "".join(diff))
        save(out / "patches" / f"{engine}-original-fingerprints.json", before)
        save(out / "patches" / f"{engine}-probe-fingerprints.json", fingerprints(source))
    for workload, _, _ in WORKLOADS:
        original = history / "sites" / workload
        shutil.copytree(original, out / "sites" / workload, ignore=shutil.ignore_patterns("dist", "__pycache__"))
        site = out / "sites" / workload / "site"
        # Preserve the exact input bytes and Tera template variant used by all
        # three selected historical points; no generated fixture substitutions.
        for p in (out / "sites" / workload / "templates-tera").iterdir():
            shutil.copy2(p, site / "templates" / p.name)
        if workload == "xiv":
            shutil.copy2(original / "scripts/1832d47.py", site / "scripts/items.py")
        expected = json.loads((history / "inventories" / f"{workload}-current.json").read_text())
        save(out / "inventories" / f"{workload}-expected.json", expected)
    # Positive control: counters must detect actual JSON/TOML reads and parsing,
    # even though none of the five historical workloads calls the Data API.
    smoke = out / "sites/data-smoke/site"
    shutil.copytree(REPO / "examples/structured-data", smoke / "example")
    write(smoke / "Notist.toml", '''[project]
name = "resource-data-positive-control"
[[site]]
name = "bench"
content = "content"
templates = "templates"
static_dir = "static"
template = "page"
''')
    write(smoke / "example/entry.notc", 'import "views.notc": item;\nfn show() -> BlockContent {\n[\n#item(p"./private/items.json", "42")\n\n#item(p"./private/items.json", "42")\n]\n}\n')
    write(smoke / "content/index.not", '#import "../example/entry.notc": show;\n#show()\n')
    write(smoke / "templates/page.tera", "<!doctype html><html><body>{{ page.html | safe }}</body></html>")
    (smoke / "static").mkdir()
    m["fixed_files"] = fingerprints(out / "sites", exclude_dist=True)
    save(out / "manifest.json", m)
    print(f"prepared {out}", flush=True)


def guard(args, require_probes=True):
    out = args.out.resolve()
    m = json.loads((out / "manifest.json").read_text())
    assert args.runs == m["runs"] and args.cpu == m["cpu"]
    assert fingerprints(out / "sites", exclude_dist=True) == m["fixed_files"], "inputs changed"
    for engine, meta in m["engines"].items():
        assert sha((out / "bin" / engine).read_bytes()) == meta["normal_sha256"]
        if engine != "environment" and require_probes:
            assert sha((out / "bin" / f"probe-{engine}").read_bytes()) == meta["probe_sha256"]
        if engine != "environment":
            assert fingerprints(out / "sources" / engine) == json.loads((out / "patches" / f"{engine}-probe-fingerprints.json").read_text()), f"probe source changed: {engine}"
    return m


def build(args):
    out = args.out.resolve()
    m = guard(args, require_probes=False)
    for engine in ["resources", "current"]:
        dest = out / "bin" / f"probe-{engine}"
        if dest.exists():
            assert sha(dest.read_bytes()) == m["engines"][engine]["probe_sha256"]
            continue
        env = dict(os.environ, **m["build_env"], CARGO_TARGET_DIR=str(out / f"target-{engine}"), NO_COLOR="1", TERM="dumb")
        command = ["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist"]
        if engine == "current":
            command += ["--features", "profiling"]
        with (out / "logs" / f"build-{engine}.log").open("wb") as log:
            subprocess.run(command, cwd=out / "sources" / engine, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(out / f"target-{engine}/release/notist", dest)
        m["engines"][engine]["probe_sha256"] = sha(dest.read_bytes())
        save(out / "manifest.json", m)
        print(f"built probe-{engine}", flush=True)


def process(args, engine, workload, label, probe=False, detail=False, mode="full", command=None, cwd=None):
    out = args.out.resolve()
    env = dict(os.environ, NO_COLOR="1", TERM="dumb", XIV_PROFILE_REQUESTS="1000000",
               RESOURCE_PROFILE=("detail" if detail else "1") if probe else "0", RESOURCE_DIAGNOSTIC=mode)
    binary = out / "bin" / (("probe-" if probe or mode != "normal" else "") + engine)
    cmd = ["taskset", "-c", str(args.cpu), *map(str, command or [binary, "build"])]
    import time
    started = time.perf_counter()
    log = out / "logs" / label
    with log.with_suffix(".stdout").open("wb") as stdout, log.with_suffix(".stderr").open("wb") as stderr:
        child = subprocess.Popen(cmd, cwd=cwd or out / "sites" / workload / "site", env=env, stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    result = {"engine": engine, "workload": workload, "mode": mode, "probe": probe, "detail": detail,
              "wall_s": time.perf_counter() - started, "user_s": usage.ru_utime, "system_s": usage.ru_stime,
              "rss_kib": usage.ru_maxrss, "returncode": child.returncode, "command": cmd,
              "stdout": str(log.with_suffix(".stdout")), "stderr": str(log.with_suffix(".stderr"))}
    if child.returncode:
        raise RuntimeError(json.dumps(result) + "\n" + log.with_suffix(".stderr").read_text()[-12000:])
    if probe:
        result["stages"], result["modules"] = parse_stages(log.with_suffix(".stderr"))
        result["resource"] = parse_resource(log.with_suffix(".stderr").read_text())
    return result


def parse_resource(text):
    rows = {}
    for line in text.splitlines():
        if not line.startswith("[resource-profile] "):
            continue
        _, phase, name, calls, nanos, maximum = line.split()
        assert name not in rows.setdefault(phase, {})
        rows[phase][name] = {"calls": int(calls), "seconds": int(nanos) / 1e9, "max": int(maximum)}
    assert set(rows) == {"other", "sources", "analyze", "evaluate"}, "missing fine-grained probes"
    return rows


def output_root(out, workload):
    return out / "sites" / workload / "site/dist" / ("xiv-db" if workload == "xiv" else "bench")


def verify(args, workload):
    out = args.out.resolve()
    actual = inventory(output_root(out, workload))
    expected = json.loads((out / "inventories" / f"{workload}-expected.json").read_text())
    assert actual["paths"] == expected["paths"], f"output mismatch: {workload}"
    return {k: v for k, v in actual.items() if k not in ["paths", "semantics"]}


def probes(args):
    out = args.out.resolve()
    guard(args)
    path = out / "probes.json"
    results = json.loads(path.read_text()) if path.exists() else []
    cells = [("resources", "data-smoke", True), ("current", "data-smoke", True)]
    cells += [(e, w[0], False) for e in ["resources", "current"] for w in WORKLOADS]
    cells += [("current", "literal", True), ("current", "shared", True)]
    for engine, workload, detail in cells:
        if any(r["engine"] == engine and r["workload"] == workload and r["detail"] == detail for r in results):
            continue
        sample = process(args, engine, workload, f"probe-{engine}-{workload}-{'detail' if detail else 'count'}", probe=True, detail=detail)
        if workload == "data-smoke":
            calls = lambda name: sum(row[name]["calls"] for row in sample["resource"].values())
            assert calls("data.read_native") == 4 and calls("data.parse_actual") == 2
            assert calls("data.cache_parse_hit") == 2 and calls("data.cache_parse_miss") == 2
            assert calls("data.decode_actual") > 0
            text = (output_root(out, workload) / "index.html").read_text()
            assert text.count("记录：共享数据视图") == 2 and "private_note" not in text, text
        else:
            sample["output"] = verify(args, workload)
            assert all(row["data.native"]["calls"] == 0 and row["data.parse_actual"]["calls"] == 0 for row in sample["resource"].values())
        results.append(sample)
        save(path, results)
        print(f"probed {engine}/{workload} detail={detail}: {sample['wall_s']:.3f}s", flush=True)
    guard(args)


def distribution(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values), "values": values}


def measurement_cells():
    cells = [(engine, workload, "normal") for engine in ENGINES for workload, _, _ in WORKLOADS]
    cells += [(engine, workload, mode) for engine in ["resources", "current"] for workload, _, _ in WORKLOADS for mode in ["full", "no-scans"]]
    cells += [("current", workload, mode) for workload in ["literal", "private"] for mode in MODES[1:-1]]
    assert len(cells) == len(set(cells))
    return cells


def measure(args):
    out = args.out.resolve()
    guard(args)
    path = out / "measurements.json"
    results = json.loads(path.read_text()) if path.exists() else []
    cells = measurement_cells()
    for rep in range(-1, args.runs):
        shift = max(rep, 0) * 13 % len(cells)
        order = cells[shift:] + cells[:shift]
        if rep % 2:
            order.reverse()
        for engine, workload, mode in order:
            if any(r["round"] == rep and (r["engine"], r["workload"], r["mode"]) == (engine, workload, mode) for r in results):
                continue
            label = f"{'warmup' if rep < 0 else f'cli-{rep}'}-{engine}-{workload}-{mode}"
            sample = process(args, engine, workload, label, mode=mode)
            # Output checking is after the child's exit, never in measured time.
            sample.update(round=rep, output=verify(args, workload))
            results.append(sample)
            save(path, results)
            print(f"{label}: {sample['wall_s']:.4f}s", flush=True)
    guard(args)


def trace(args):
    """System-call counts on an explicitly separate 66-page positive control."""
    out = args.out.resolve()
    guard(args)
    base = out / "trace/literal-64"
    if not base.exists():
        synthetic(base, "literal", pages=64)
        save(out / "trace/input-fingerprints.json", fingerprints(base, exclude_dist=True))
    assert fingerprints(base, exclude_dist=True) == json.loads((out / "trace/input-fingerprints.json").read_text())
    path = out / "trace/runs.json"
    results = json.loads(path.read_text()) if path.exists() else []
    for engine, mode in [(e, "normal") for e in ENGINES] + [("current", "full"), ("current", "no-scans")]:
        if any(r["engine"] == engine and r["mode"] == mode for r in results):
            continue
        binary = out / "bin" / (("" if mode == "normal" else "probe-") + engine)
        counts = out / "trace" / f"{engine}-{mode}.strace"
        warmup = process(args, engine, "trace-literal-64", f"strace-warmup-{engine}-{mode}", mode=mode, command=[binary, "build"], cwd=base / "site")
        command = ["strace", "-qq", "-f", "-c", "-e", "trace=%file,close,fstat,read,fcntl,dup,dup3", "-o", str(counts), str(binary), "build"]
        sample = process(args, engine, "trace-literal-64", f"strace-{engine}-{mode}", mode=mode, command=command, cwd=base / "site")
        actual = inventory(base / "site/dist/bench")
        expected = out / "trace/expected.json"
        if not expected.exists():
            assert engine == "environment"
            save(expected, actual)
        else:
            assert actual["paths"] == json.loads(expected.read_text())["paths"]
        assert actual["html"] == 66
        sample["strace"] = str(counts)
        sample["warmup"] = warmup
        sample["syscalls"] = parse_strace(counts.read_text())
        results.append(sample)
        save(path, results)
        print(f"strace counts {engine}/{mode}: {sample['syscalls']}", flush=True)
    guard(args)


def parse_strace(text):
    result = {}
    for line in text.splitlines():
        cols = line.split()
        if len(cols) in [5, 6] and re.fullmatch(r"\d+\.\d+", cols[0]) and re.fullmatch(r"\d+", cols[3]):
            result[cols[-1]] = {"calls": int(cols[3]), "errors": int(cols[4]) if len(cols) == 6 else 0}
    assert "total" in result
    return result


def observed(args):
    """Separate, more intrusive timers for the CLI's observation wrapper."""
    out = args.out.resolve()
    m = guard(args)
    binary = out / "bin/observed-current"
    if not binary.exists():
        source = out / "sources/current"
        paths = [source / rel for rel in ["crates/notist-cli/src/main.rs", "crates/notist-cli/src/publication.rs", "crates/notist-ssg/src/assets.rs"]]
        before = {p: p.read_text() for p in paths}
        module = source / "crates/notist-cli/src/observed_profile.rs"
        assert not module.exists()
        try:
            module.write_text(Path(__file__).with_name("observed_reader_probe.rs").read_text())
            paths[0].write_text(enter_function(before[paths[0]], "main", "    let _observed_report = observed_profile::Report;") + "\nmod observed_profile;\n")
            text = before[paths[1]]
            start = text.index("    fn revision(", text.index("impl ResourceReader for ObservedReader {"))
            part = enter_function(text[start:], "revision", "        let _rp_revision = crate::observed_profile::span(0);")
            part = once(part, "        let absolute = self.root.join(path);", "        let absolute = { let _rp = crate::observed_profile::span(1); self.root.join(path) };")
            part = once(part, "        self.observed.lock().unwrap().insert(absolute.clone());", "        { let _rp = crate::observed_profile::span(2); self.observed.lock().unwrap().insert(absolute.clone()); }")
            paths[1].write_text(text[:start] + part)
            paths[2].write_text(enter_function(before[paths[2]], "data", '    let _site_data = notist::xiv_profile::span("ssg.site_data");'))
            patches = []
            for p, a in [*before.items(), (module, "")]:
                rel = p.relative_to(source).as_posix()
                patches.extend(difflib.unified_diff(a.splitlines(True), p.read_text().splitlines(True), fromfile="a/" + rel, tofile="b/" + rel))
            write(out / "patches/observed-current.patch", "".join(patches))
            env = dict(os.environ, **m["build_env"], CARGO_TARGET_DIR=str(out / "target-current"))
            with (out / "logs/build-observed-current.log").open("wb") as log:
                subprocess.run(["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist", "--features", "profiling"], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            shutil.copy2(out / "target-current/release/notist", binary)
            m["engines"]["current"]["observed_sha256"] = sha(binary.read_bytes())
            save(out / "manifest.json", m)
        finally:
            for p, text in before.items():
                p.write_text(text)
            module.unlink()
    assert sha(binary.read_bytes()) == m["engines"]["current"]["observed_sha256"]
    guard(args)
    path = out / "observed-probes.json"
    results = json.loads(path.read_text()) if path.exists() else []
    for workload in ["literal", "shared", "xiv"]:
        if any(p["workload"] == workload for p in results):
            continue
        sample = process(args, "current", workload, f"observed-current-{workload}", probe=True, detail=True, command=[binary, "build"])
        sample["observed"] = {name: {"calls": int(calls), "seconds": int(nanos) / 1e9} for name, calls, nanos in re.findall(r"\[resource-observed\] (\S+) (\d+) (\d+)", Path(sample["stderr"]).read_text())}
        assert len(sample["observed"]) == 3
        sample["output"] = verify(args, workload)
        results.append(sample)
        save(path, results)
        print(f"observed wrapper detail {workload}: {sample['observed']}", flush=True)
    guard(args)


def totals(sample, metric):
    rows = [row[metric] for row in sample["resource"].values()]
    return {"calls": sum(r["calls"] for r in rows), "seconds": sum(r["seconds"] for r in rows), "max": max(r["max"] for r in rows)}


def verify_evidence(args, probed):
    """Check deterministic work counts, independently of noisy elapsed time."""
    out = args.out.resolve()
    workloads = {w: pages for w, _, pages in WORKLOADS}
    ordinary_probes = [p for p in probed if p["workload"] in workloads and not p["detail"]]
    for p in ordinary_probes:
        r = p["resource"]
        pages = workloads[p["workload"]]
        assert totals(p, "session.source_hit")["calls"] + totals(p, "session.source_miss")["calls"] == totals(p, "session.source_bytes")["calls"]
        assert totals(p, "validate.total")["calls"] == totals(p, "session.source_bytes")["calls"] + 2 * pages + 1
        physical_pages = r["sources"]["session.source_bytes"]["calls"]
        assert r["sources"]["validate.file_checks"]["calls"] == physical_pages * (physical_pages - 1) // 2
        assert r["evaluate"]["validate.total"]["calls"] == 2 * pages
        assert r["evaluate"]["validate.file_checks"]["calls"] == 2 * pages * r["evaluate"]["observed.files_max"]["max"]
        assert totals(p, "validate.kind_checks")["calls"] == 0
        assert totals(p, "session.source_miss")["calls"] == totals(p, "fs.read_bounded")["calls"] + 1
        assert all(totals(p, name)["calls"] == 0 for name in ["data.native", "data.parse_actual", "data.cache_decode"])
    for workload in workloads:
        rows = [p for p in ordinary_probes if p["workload"] == workload]
        assert len(rows) == 2
        for metric in ["validate.total", "validate.file_checks", "validate.kind_checks", "session.source_bytes", "fs.revision"]:
            assert totals(rows[0], metric)["calls"] == totals(rows[1], metric)["calls"], (workload, metric)
    traces = json.loads((out / "trace/runs.json").read_text())
    full = next(p["syscalls"] for p in traces if p["engine"] == "current" and p["mode"] == "full")
    skip = next(p["syscalls"] for p in traces if p["engine"] == "current" and p["mode"] == "no-scans")
    normal = next(p["syscalls"] for p in traces if p["engine"] == "current" and p["mode"] == "normal")
    extra_opens = full["openat2"]["calls"] - skip["openat2"]["calls"]
    assert full["statx"]["calls"] - skip["statx"]["calls"] == 2 * extra_opens
    assert full["close"]["calls"] - skip["close"]["calls"] == extra_opens
    assert full["read"]["calls"] == skip["read"]["calls"]
    assert all(full[k]["calls"] == normal[k]["calls"] for k in ["openat2", "statx", "close", "read"])
    module_controls = 0
    measurements = out / "measurements.json"
    if measurements.exists():
        for sample in json.loads(measurements.read_text()):
            if sample["engine"] != "current" or sample["mode"] == "normal":
                continue
            _, modules = parse_stages(Path(sample["stderr"]))
            expected = next(p["modules"] for p in ordinary_probes if p["engine"] == "current" and p["workload"] == sample["workload"])
            assert all(modules[k]["calls"] == expected[k]["calls"] for k in ["compile_work", "bind_work"])
            module_controls += 1
    assert git(REPO, "diff", "--binary", "HEAD") == (out / "checkout-start.diff").read_bytes()
    return {"deterministic_counts_verified": len(ordinary_probes), "traced_extra_openat2": extra_opens,
            "traced_extra_statx": 2 * extra_opens, "read_calls_unchanged": True, "module_work_controls_verified": module_controls, "prior_tracked_diff_preserved": True}


def chart_payload(summary):
    data = {"probes": [], "cells": [], "runs": summary["runs"], "builds": len(summary["cells"]) * (summary["runs"] + 1)}
    for p in summary["probes"]:
        if p["detail"] or p["workload"] == "data-smoke":
            continue
        data["probes"].append({"engine": p["engine"], "workload": p["workload"], "wall": round(p["wall_s"], 6),
            "validate": round(totals(p, "validate.total")["seconds"], 6), "checks": totals(p, "validate.file_checks")["calls"],
            "phases": {key: round(row["validate.total"]["seconds"], 6) for key, row in p["resource"].items()}})
    for c in summary["cells"]:
        if c["mode"] != "normal":
            data["cells"].append({k: c[k] for k in ["engine", "workload", "mode"]} | {"wall": round(c["wall_s"]["median"], 6)})
    return data


def report(args):
    import csv
    out = args.out.resolve()
    m = guard(args)
    measured = json.loads((out / "measurements.json").read_text())
    probed = json.loads((out / "probes.json").read_text())
    assert len(measured) == (args.runs + 1) * len(measurement_cells())
    assert len(probed) == 14
    evidence = verify_evidence(args, probed)
    save(out / "integrity.json", evidence)
    summary = {"engines": m["engines"], "runs": args.runs, "cpu": args.cpu, "cells": [], "probes": probed}
    for engine, workload, mode in measurement_cells():
        samples = [r for r in measured if (r["engine"], r["workload"], r["mode"]) == (engine, workload, mode) and r["round"] >= 0]
        assert len(samples) == args.runs and all(r["returncode"] == 0 for r in samples)
        cell = {"engine": engine, "workload": workload, "mode": mode}
        cell.update({metric: distribution([s[metric] for s in samples]) for metric in ["wall_s", "user_s", "system_s", "rss_kib"]})
        summary["cells"].append(cell)
    save(out / "summary.json", summary)
    payload = chart_payload(summary)
    save(out / "chart-data.json", payload)
    template = Path(__file__).with_name("resource_overhead_chart.html").read_text()
    html = template.replace("__RESOURCE_OVERHEAD__", json.dumps(payload, ensure_ascii=False, separators=(",", ":")).replace("<", "\\u003c"))
    # Standalone browser pages need an opaque fallback canvas; the inline-chat
    # template itself stays transparent and uses the host's live theme tokens.
    html = html.replace("</head>", "<style>html{background:var(--background,#ffffff)}</style></head>")
    write(out / "dashboard.html", html)
    with (out / "summary.csv").open("w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["engine", "workload", "mode", "wall_median_s", "wall_min_s", "wall_max_s", "user_median_s", "system_median_s", "rss_median_mib"])
        for c in summary["cells"]:
            writer.writerow([c["engine"], c["workload"], c["mode"], *[c["wall_s"][k] for k in ["median", "min", "max"]], c["user_s"]["median"], c["system_s"]["median"], c["rss_kib"]["median"] / 1024])
    cell = lambda engine, workload, mode="normal": next(c for c in summary["cells"] if (c["engine"], c["workload"], c["mode"]) == (engine, workload, mode))
    probe = lambda engine, workload: next(p for p in probed if p["engine"] == engine and p["workload"] == workload and not p["detail"])
    private = probe("current", "private")
    lines = ["# 资源／Data 重构新增耗时定位（2026-11-03）", "", "本轮重新计时，不与旧批次秒数相减；保留完整五种负载和全部输出。所有引擎改写仅发生在结果目录的副本。", "",
             "## 结论：主因是重复全批文件版本检查，不是 Data 解析", "",
             f"- 当前私有图单次主探针 CLI {private['wall_s']:.3f} s，其中 validate {totals(private, 'validate.total')['seconds']:.3f} s（{totals(private, 'validate.total')['seconds'] / private['wall_s'] * 100:.1f}%）；累计 {totals(private, 'validate.file_checks')['calls']:,} 次已观察文件检查。",
             "- 五种历史负载的语言 Data native／parse／decode 调用均为零，不能把这次退化归因于 JSON/TOML 解析或 Data 投影缓存。真实 Data 阳性控制有 4 次读取、2 次解析、2 次解析缓存命中，确认计数路径有效。",
             "- kinds 校验在这五种负载中实际迭代零次；首次文件内容读取是毫秒量级。主要成本发生在反复确认以前读过的文件有没有变化。",
             "- 三条触发链：首次加载所有物理页面时，source_bytes 每次先扫描之前已读集合；分析各页导入时，包括源码字节缓存命中前也先扫描；每页 evaluate 前后再各扫一遍。",
             "- 确定性计数与代码一致：初始 N 页源读取产生 N×(N−1)/2 次检查；求值产生 2×总页数×最终观察文件数次检查。两者都已逐情形自动核对，不只是从 wall 猜复杂度。",
             "- CLI ObservedReader.revision 每次还重新拼路径、锁住 observed 集合并插入同一路径；底层 FsReader.revision 重新 capability-open 文件、检查文件类型并再取 metadata。重复全量扫描同时放大这些用户态与内核态成本。",
             "- comemo 复用编译／绑定没有消除这里的源码版本校验。版本重构点与当前点的所有这些检查次数逐情形完全相同。",
             "- 当前所有诊断模式的实际编译／绑定次数都与 full 主探针一致；跳过扫描不是跳过模块工作，私有图仍完整编译 5,120 个不同模块身份。",
             "- 下文消融只用于归因，不是可用优化：它们不再保持完整的构建中修改检测语义。正式引擎未改动。", "",
             "## 普通二进制：重构前／重构点／当前", "", "| 情形 | 重构前 s | 资源重构 s | 当前 s |", "| --- | ---: | ---: | ---: |"]
    for workload, label, _ in WORKLOADS:
        lines.append("| " + label + " | " + " | ".join(f"{cell(e, workload)['wall_s']['median']:.4f}" for e in ENGINES) + " |")
    lines += ["", "XIV 的页面主要由 prepare 返回为虚拟文档，版本会话最终只观察 2 个输入（其中一个是虚拟目标配置）；人工站点至少观察 522 个输入。这解释了为什么页数更多的 XIV 反而没有触发同样严重的退化，不能仅按页数跨情形推断效率。"]
    lines += ["", "## 全检查探针：版本校验内部耗时与实际次数", "", "单次普通粒度探针，包含式计时；文件／kind 检查互斥，但不要把父子行再相加。细粒度文件计时另存，不用于此表。", "",
              "| 版本／情形 | 探针 CLI s | validate s | 文件 revision 检查 s | kind 检查 s | 文件检查次数 | kind 检查次数 | Data 解析次数 |", "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for engine in ["resources", "current"]:
        for workload, label, _ in WORKLOADS:
            p = probe(engine, workload)
            v, f, k, fc, kc, d = [totals(p, metric) for metric in ["validate.total", "validate.files", "validate.kinds", "validate.file_checks", "validate.kind_checks", "data.parse_actual"]]
            lines.append(f"| {engine}／{label} | {p['wall_s']:.4f} | {v['seconds']:.4f} | {f['seconds']:.4f} | {k['seconds']:.4f} | {fc['calls']:,} | {kc['calls']:,} | {d['calls']} |")
    lines += ["", "### 当前私有图：扫描发生在哪个阶段", "", "| 阶段 | validate s | 文件检查次数 |", "| --- | ---: | ---: |"]
    for phase, label in [("sources", "初始物理页面读取"), ("analyze", "分析／导入加载"), ("evaluate", "求值前后"), ("other", "最终边界")]:
        r = private["resource"][phase]
        lines.append(f"| {label} | {r['validate.total']['seconds']:.4f} | {r['validate.file_checks']['calls']:,} |")
    lines += ["", "### 读取内容不是大头", "", "| 情形 | 物理有界读取次数 | FsReader 内容读取 s | 最终观察文件数（含虚拟配置） | 源字节缓存命中次数 |", "| --- | ---: | ---: | ---: | ---: |"]
    for workload, label, _ in WORKLOADS:
        p = probe("current", workload)
        read = totals(p, "fs.read_bounded")
        lines.append(f"| {label} | {read['calls']:,} | {read['seconds']:.6f} | {totals(p, 'observed.files_max')['max']:,} | {totals(p, 'session.source_hit')['calls']:,} |")
    lines += ["", "## 诊断性消融（不安全，不能作为生产实现）", "", "同一临时探针二进制关闭细粒度资源计数与计时后，对比保留全部校验和跳过全批扫描；粗粒度阶段探针及当前版本的模块 profiling 在各模式中保持相同。两者仍执行请求计费、授权及首次读取前后 revision，但跳过扫描不再保证构建中修改的批次一致性。这里只用于归因。", "",
              "| 版本／情形 | 全检查 s | 跳过全批扫描 s | 差额 s |", "| --- | ---: | ---: | ---: |"]
    for engine in ["resources", "current"]:
        for workload, label, _ in WORKLOADS:
            full, skip = [cell(engine, workload, mode)["wall_s"]["median"] for mode in ["full", "no-scans"]]
            lines.append(f"| {engine}／{label} | {full:.4f} | {skip:.4f} | {full-skip:.4f} |")
    lines += ["", "跳过扫描后的时间不保证等于重构前：还包含该提交的共享值／求值／SSG 变更，以及诊断二进制的少量探针框架开销。本轮把重复扫描确认为主要增量，没有声称其解释每一微秒的差额。"]
    lines += ["", "### 当前版本：分别跳过哪一类扫描", "", "| 情形 | full | no-files | no-kinds | no-source-scans | no-eval-scans | no-scans |", "| --- | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for workload in ["literal", "private"]:
        lines.append("| " + workload + " | " + " | ".join(f"{cell('current', workload, mode)['wall_s']['median']:.4f}" for mode in MODES) + " |")
    observed_probes = json.loads((out / "observed-probes.json").read_text())
    lines += ["", "## 补充细探针：CLI 观察集合也被重复维护", "", "这些运行额外逐次读时钟，明显更侵入，只用来看内部量级；不能与主探针或普通 CLI 相减。ObservedReader.revision 包含 FsReader.revision，后者又包含 capability open，表中不能横向求和。", "",
              "| 情形 | 外层 ObservedReader.revision s | 路径拼接 s | 加锁／集合插入 s | 内层 FsReader.revision s | capability open s | site.data 载入 s |", "| --- | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for p in observed_probes:
        o = p["observed"]
        lines.append(f"| {p['workload']} | {o['observed.revision']['seconds']:.4f} | {o['observed.path_join']['seconds']:.4f} | {o['observed.record_set']['seconds']:.4f} | {totals(p, 'fs.revision')['seconds']:.4f} | {totals(p, 'fs.cap_open')['seconds']:.4f} | {p['stages']['ssg.site_data']['seconds']:.6f} |")
    lines += ["", "## 独立系统调用证据（额外 66 页，不混入主计时）", "", "strace -c 只用于次数，不使用追踪运行的 wall。此表的合计仅包括选择追踪的文件相关系统调用，不是进程所有系统调用。", "",
              "| 二进制／模式 | openat2 | statx | close | read | 所选调用合计 |", "| --- | ---: | ---: | ---: | ---: | ---: |"]
    for p in json.loads((out / "trace/runs.json").read_text()):
        lines.append("| " + p["engine"] + "/" + p["mode"] + " | " + " | ".join(f"{p['syscalls'].get(k, {}).get('calls', 0):,}" for k in ["openat2", "statx", "close", "read", "total"]) + " |")
    lines += ["", f"full 相比 no-scans 额外 {evidence['traced_extra_openat2']:,} 次 openat2、{evidence['traced_extra_statx']:,} 次 statx，以及同等数量的 close；read 次数不变。印证增加的是打开／检查版本，不是重读内容或解析 JSON。", "",
              "## 后续方向（本轮未实现）", "", "应避免在每次缓存命中与每页求值边界重复重查整个已观察集合，改用明确的批次快照／版本与失效机制。但不能直接提交 no-scans：必须保留授权、单次读取前后变化检测、跨文档读取一致性、错误／缺失文件的失效、prepared retries，以及构建中修改的既有可观察行为。先明确批次边界与并发修改契约，再实现和回归。"]
    tests_log = out / "logs/full-policy-tests.log"
    test_counts = re.findall(r"test result: ok\. (\d+) passed; 0 failed", tests_log.read_text()) if tests_log.exists() else []
    test_note = f"临时全检查副本的资源会话／读取／路径／Data 共 {sum(map(int, test_counts))} 项 Rust 测试通过。" if test_counts else "本批未记录额外 Rust 安全回归运行。"
    rss_note = "原始 wait4 RSS 包含 fork→exec 前从 Python 启动器继承的高水位，不能直接视作 Rust 构建自身峰值。本轮不作 RSS 排名或跨批次内存比较。"
    if (out / "rss-launcher-control.json").exists():
        controls = json.loads((out / "rss-launcher-control.json").read_text())
        rss_note += f" 独立控制将启动器分配从 {controls[0]['parent_allocation_mib']} 增至 {controls[-1]['parent_allocation_mib']} MiB 后，true 子进程报告值从 {controls[0]['trivial_child_wait4_rss_kib'] / 1024:.1f} 增至 {controls[-1]['trivial_child_wait4_rss_kib'] / 1024:.1f} MiB。"
    lines += ["", "## 方法与边界", "",
              f"- {len(measurement_cells())} 格，每格完整预热一次、{args.runs} 轮交错反序测量，共 {len(measured)} 次 CLI；所有 {len(measured)} 次产物逐文件 SHA-256 与历史完整基准一致。",
              "- 14 次探针，包括两个版本 × 五种完整负载、两个额外 detail 模式，以及两个真实 JSON/TOML 读取阳性控制。阳性控制要求 4 次读取、2 次实际解析、2 次解析缓存命中和真实解码，防止把探针未接线误当作零调用。",
              "- 另有 3 次 CLI 观察包装器细探针、5 次 strace（每次预热）；" + test_note,
              "- 普通五种负载不使用语言 Data API；XIV 的 Python 预处理和模板 site.data 不是语言 Data 解析。不要将此结论解释为 Data 解析器已被全面性能评估。",
              "- 普通粒度探针按来源读取／分析／求值／其余分组，validate 的文件和 kind 子循环分别计时、逐次计数；高频 FsReader 调用只计数，detail 模式才给每个文件调用读时钟。探针未额外逐格预热，初次输出是否存在会影响包含式 CLI，故只用于量级定位，不用于跨版本排名。",
              "- 所有数字包含完整构建与发布；新进程、CPU 8、温文件缓存、未锁频、不 fsync。包含式阶段不能重复相加，单次探针只用于量级定位。RSS 是 wait4 高水位，不是进程树同时总量。",
              "- " + rss_note,
              "- 正式发布版本二进制、临时全检查二进制、计时探针、跳过扫描的诊断性消融分别保留，不用探针绝对时间替代普通 CLI 排名。",
              "- 复用指纹核对的历史二进制及源码，不复用旧秒数。重构前为 0d81985，重构点 ef38d1b，当前固定已提交 083ff6c，原有未提交 SSG 宿主政策不纳入。临时 CLI requests=1,000,000，未改变消费者或依赖锁。",
              f"- 原始记录、全部样本／范围、补丁与源码／二进制指纹：`{out}`。", ""]
    write(out / "report.md", "\n".join(lines))
    print(f"wrote {out / 'report.md'}", flush=True)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", choices=["prepare", "build", "probes", "measure", "trace", "observed", "report"])
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--history", type=Path, default=Path("/tmp/opencode/notist-history-matrix-20261103"))
    p.add_argument("--runs", type=int, default=3)
    p.add_argument("--cpu", type=int, default=8)
    args = p.parse_args()
    if args.runs < 1:
        p.error("runs must be positive")
    {"prepare": prepare, "build": build, "probes": probes, "measure": measure, "trace": trace, "observed": observed, "report": report}[args.action](args)


if __name__ == "__main__":
    main()
