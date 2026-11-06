#!/usr/bin/env python3
"""Frozen-workload evaluator experiments; production files are never rewritten."""
import argparse
import difflib
import json
import os
from pathlib import Path
import random
import shutil
import statistics
import subprocess
import threading
import time

from profile_icexiv import inventory, instrument as coarse_instrument
from profile_xiv_history import REPO, once, save, sha
from profile_current import parse_detail
from xiv_history_probes import enter_function

KINDS = ["base", "instance", "signature", "scope", "present", "combined"]
EVAL = "crates/notist-pipeline/src/code/eval.rs"
ENV = {"CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "2", "CARGO_PROFILE_RELEASE_DEBUG": "1"}


def edit(source, relative, fn):
    path = source / relative
    path.write_text(fn(path.read_text()))


def instance(text):
    text = once(text, "    context: &'a Context,\n    program: CheckedProgram,", "    context: &'a Context,\n    implementation_cache: Vec<(FunctionInstance, Arc<InstanceRecord>)>,\n    program: CheckedProgram,")
    text = text.replace("        program: program.clone(),", "        implementation_cache: Vec::new(),\n        program: program.clone(),")
    text = once(text, "                        program: analysis.program.clone(),", "                        implementation_cache: Vec::new(),\n                        program: analysis.program.clone(),")
    # Only Type arguments use PartialEq: floats/data keep their exact original
    # canonical-key path. Never cache errors or provisional checking records.
    anchor = "        let key = instance.key();"
    text = once(text, anchor, '''        let reusable = instance.arguments.iter().all(|v| matches!(v, ComptimeValue::Type(_)));
        if reusable {
            if let Some((_, record)) = self.implementation_cache.iter().find(|(key, _)| key == instance) {
                return Ok(record.clone());
            }
        }
''' + anchor)
    return once(text, "        Ok(record.clone())\n    }", '''        if reusable && !record.checking {
            self.implementation_cache.push((instance.clone(), record.clone()));
        }
        Ok(record.clone())
    }''')


def signature(source):
    edit(source, "crates/notist-core/src/evaluation.rs", lambda t: once(t,
        "    pub definition: Box<FunctionDef>,", '''    #[cfg_attr(feature = "serde", serde(with = "shared_definition"))]
    pub definition: std::sync::Arc<FunctionDef>,''') + '''
#[cfg(feature = "serde")]
mod shared_definition {
    use super::FunctionDef;
    use serde::{Serialize, Deserialize, Serializer, Deserializer};
    use std::sync::Arc;
    pub fn serialize<S: Serializer>(value: &Arc<FunctionDef>, s: S) -> Result<S::Ok, S::Error> { value.as_ref().serialize(s) }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Arc<FunctionDef>, D::Error> { FunctionDef::deserialize(d).map(Arc::new) }
}
''')
    edit(source, EVAL, lambda t: once(t, "            definition: Box::new(def),", "            definition: Arc::new(def),"))
    edit(source, "crates/notist-pipeline/src/code/check.rs", lambda t: once(t, "            definition: Box::new(concrete),", "            definition: Arc::new(concrete),"))


SCOPE = '''#[derive(Clone, Default)]
struct Scope(Option<Arc<ScopeFrame>>);
#[derive(Clone, Default)]
struct ScopeFrame {
    parent: Scope,
    values: BTreeMap<String, Option<Runtime>>,
}
impl Scope {
    fn new() -> Self { Self::default() }
    fn fork(&self) -> Self {
        if self.0.is_none() { return Self::new(); }
        Self(Some(Arc::new(ScopeFrame { parent: self.clone(), values: BTreeMap::new() })))
    }
    fn get(&self, key: &str) -> Option<&Runtime> {
        let frame = self.0.as_ref()?;
        match frame.values.get(key) {
            Some(value) => value.as_ref(),
            None => frame.parent.get(key),
        }
    }
    fn frame(&mut self) -> &mut ScopeFrame { Arc::make_mut(self.0.get_or_insert_with(|| Arc::new(ScopeFrame::default()))) }
    fn insert(&mut self, key: String, value: Runtime) { self.frame().values.insert(key, Some(value)); }
    fn remove(&mut self, key: &str) { if self.get(key).is_some() { self.frame().values.insert(key.into(), None); } }
    fn contains_key(&self, key: &str) -> bool { self.get(key).is_some() }
    // Conservative empty fast path; tombstones may make an empty frame look
    // nonempty, never the reverse. Only used to skip capture traversal.
    fn is_empty(&self) -> bool { self.0.as_ref().is_none_or(|frame| frame.values.is_empty() && frame.parent.is_empty()) }
}
'''


def scope(text):
    text = once(text, "type Scope = BTreeMap<String, Runtime>;", SCOPE)
    assert text.count("let mut local = scope.clone();") == 5
    return text.replace("let mut local = scope.clone();", "let mut local = scope.fork();")


def present(text):
    head, body = text.split("    fn present(", 1)
    body, tail = body.split("    fn present_node(", 1)
    body = once(body, "        let mut result = Vec::with_capacity(forest.len());\n", "")
    body = once(body, "        for node in forest {", "        let result = forest.into_iter().map(|node| {")
    body = once(body, '                result.push(self.error(boundary, "content size limit exceeded".into()));\n                summary.failed = true;\n                continue;', '                summary.failed = true;\n                return self.error(boundary, "content size limit exceeded".into());')
    body = once(body, "            result.push(item);\n        }", "            item\n        }).collect();")
    return head + "    fn present(" + body + "    fn present_node(" + tail


def variant(source, kind):
    if kind in ["instance", "combined"]:
        edit(source, EVAL, instance)
    if kind in ["signature", "combined"]:
        signature(source)
    if kind in ["scope", "combined"]:
        edit(source, EVAL, scope)
    if kind in ["present", "combined"]:
        edit(source, EVAL, present)


def observer(source):
    coarse_instrument(source, False)
    # Keep production automatic SSG drop order, rather than moving deallocation
    # costs between named objects.
    def site(t):
        for name in ["values", "dirs", "components", "by_source", "engine", "documents", "handles"]:
            t = once(t, f'    notist::xiv_profile::measure("drop.{name}", || drop({name}));\n', "")
        return once(t, 'notist::xiv_profile::measure("pipeline.evaluate", || vault.evaluate_document(&handle, &analysis))',
            'notist::xiv_profile::measure("pipeline.evaluate", || { let _page = notist::eval_probe::page(&route.source); vault.evaluate_document(&handle, &analysis) })')
    edit(source, "crates/notist-ssg/src/build.rs", site)
    shutil.copy2(REPO / "tools/evaluator_ab_probe.rs", source / "crates/notist-core/src/eval_probe.rs")
    edit(source, "crates/notist-core/src/lib.rs", lambda t: t + "\n#[doc(hidden)]\npub mod eval_probe;\n")
    edit(source, "src/lib.rs", lambda t: t + "\n#[doc(hidden)]\npub use notist_core::eval_probe;\n")
    edit(source, "crates/notist-cli/src/main.rs", lambda t: once(t, "let _report = notist::xiv_profile::Report;", "notist::eval_probe::init();\nlet _eval_report = notist::eval_probe::Report;\nlet _report = notist::xiv_profile::Report;"))
    p = "notist_core::eval_probe"
    def instrument(t):
        for name, label in [("run", 0), ("expr", 1), ("bind_content", 2), ("present", 3), ("present_call", 4), ("execute", 5), ("implementation", 6), ("capture_call", 7), ("capture_expr", 8), ("item_shell", 9), ("expand", 10), ("content_count", 11), ("valid_structure", 12)]:
            t = enter_function(t, name, f"let _eval_observe = {p}::span({label});")
        t = once(t, '        let target = FunctionInstance {', f'        let args_observe = {p}::span(18);\n        let target = FunctionInstance {{')
        t = once(t, '        let implementation = self.implementation(&target)?;', '        drop(args_observe);\n        let implementation = self.implementation(&target)?;')
        t = once(t, '        let result = match definition.implementation {', f'        let dispatch_observe = {p}::span(19);\n        let result = match definition.implementation {{')
        t = once(t, '        if value_call {\n            self.depth -= 1;', f'        drop(dispatch_observe);\n        let _adapt_observe = {p}::span(20);\n        if value_call {{\n            self.depth -= 1;')
        t = once(t, '    if !e.needed.is_empty() {', f'    if {p}::counting() {{\n        fn output_nodes(item: &Item) -> usize {{ 1 + item.children.iter().map(output_nodes).sum::<usize>() }}\n        {p}::count(22, roots.iter().map(output_nodes).sum());\n    }}\n    if !e.needed.is_empty() {{')
        t = once(t, '        let mut inputs = Scope::new();', f'        {p}::count(if code {{ 27 }} else {{ 26 }}, 1);\n        let mut inputs = Scope::new();')
        # Direct counts disambiguate forest function entries from actual nodes.
        for name, code in [("bind_content", f"{p}::count(23, forest.len());"), ("present", f"{p}::count(24, forest.len()); if forest.is_empty() {{ {p}::count(25, 1); }}")]:
            t = enter_function(t, name, code)
        for operation in ["scope.clone()", "scope.fork()"]:
            t = t.replace(f"let mut local = {operation};", f"let mut local = {p}::measure(17, || {operation});")
        t = t.replace("Arc::unwrap_or_clone(nodes)", f"{p}::cow(nodes)").replace("Arc::unwrap_or_clone(forest)", f"{p}::cow(forest)")
        t = once(t, "        let key = instance.key();", f"        let key = instance.key();\n        {p}::count(30, key.len());")
        if "self.implementation_cache.iter()" in t:
            t = once(t, "                return Ok(record.clone());", f"                {p}::count(31, 1);\n                return Ok(record.clone());")
        t = once(t, "ExprKind::Input(name) => scope.get(name).cloned().ok_or_else(|| {", f"ExprKind::Input(name) => {p}::measure(21, || scope.get(name).cloned()).ok_or_else(|| {{")
        return t
    edit(source, EVAL, instrument)
    for relative, methods in [
        ("crates/notist-pipeline/src/code/data.rs", [("data_native", 13), ("read_data", 14)]),
        ("crates/notist-core/src/data/parse.rs", [("parse", 15)]),
        ("crates/notist-core/src/data/mod.rs", [("decode", 16)]),
    ]:
        prefix = p if "notist-pipeline" in relative else "crate::eval_probe"
        for name, label in methods:
            edit(source, relative, lambda t, name=name, label=label: enter_function(t, name, f"let _eval_observe = {prefix}::span({label});"))


def patch_between(original, changed):
    patch = []
    for path in sorted(changed.rglob("*.rs")):
        rel = path.relative_to(changed)
        before = (original / rel).read_text() if (original / rel).exists() else ""
        patch.extend(difflib.unified_diff(before.splitlines(True), path.read_text().splitlines(True), fromfile="a/" + str(rel), tofile="b/" + str(rel)))
    return "".join(patch)


def prepare(args):
    out, old = args.out, args.base
    assert not out.exists(), "fresh external directory required"
    assert not out.is_relative_to(REPO)
    old_m = json.loads((old / "manifest.json").read_text())
    assert inventory(old / "sources/normal") == old_m["sources"]["normal"]
    out.mkdir(parents=True)
    for name in ["sources", "bin", "logs", "patches", "reproducer"]:
        (out / name).mkdir()
    shutil.copytree(old / "site", out / "site", ignore=shutil.ignore_patterns("dist", "__pycache__"))
    assert inventory(out / "site") == old_m["inputs"]
    shutil.copy2(old / "output-inventory.json", out / "output-inventory.json")
    m = {"revision": old_m["revision"], "base": str(old), "inputs": old_m["inputs"],
         "rounds": args.rounds, "build_env": ENV, "source_hashes": {}, "binaries": {},
         "affinity": sorted(os.sched_getaffinity(0)), "rustc": subprocess.check_output(["rustc", "-Vv"], text=True)}
    for kind in KINDS:
        source = out / "sources" / kind
        shutil.copytree(old / "sources/normal", source)
        variant(source, kind)
        (out / "patches" / f"{kind}.patch").write_text(patch_between(out / "sources/base", source))
        m["source_hashes"][kind] = inventory(source)
    for kind in ["base-probe", "combined-probe"]:
        source = out / "sources" / kind
        original = out / "sources" / kind.removesuffix("-probe")
        shutil.copytree(original, source)
        observer(source)
        (out / "patches" / f"{kind}.patch").write_text(patch_between(original, source))
        m["source_hashes"][kind] = inventory(source)
    for name in ["profile_evaluator_ab.py", "evaluator_ab_probe.rs", "evaluator_ab_alloc.c", "test_profile_evaluator_ab.py", "EVALUATOR_AB.md", "current_profile_probe.rs", "profile_icexiv.py", "profile_current.py", "profile_xiv_history.py", "xiv_history_probes.py", "profile_history_matrix.py", "profile_resource_overhead.py", "profile_module_site.py"]:
        shutil.copy2(REPO / "tools" / name, out / "reproducer" / name)
    m["reproducer"] = inventory(out / "reproducer")
    save(out / "manifest.json", m)
    print("prepared", m["revision"], flush=True)


def build(args):
    out = args.out
    m = json.loads((out / "manifest.json").read_text())
    for kind in m["source_hashes"]:
        assert inventory(out / "sources" / kind) == m["source_hashes"][kind]
        if kind in m["binaries"]:
            assert sha((out / "bin" / kind).read_bytes()) == m["binaries"][kind]
            continue
        source = out / "sources" / kind
        env = dict(os.environ, **ENV, CARGO_TARGET_DIR=str(out / "target"))
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=source, env=env))
        packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
        with (out / "logs" / f"build-{kind}.log").open("wb") as log:
            subprocess.run(["cargo", "clean", "--release", *[flag for p in packages for flag in ["-p", p]]], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            command = ["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist"]
            if kind.endswith("-probe"):
                command += ["--features", "profiling"]
            subprocess.run(command, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            shutil.copy2(out / "target/release/notist", out / "bin" / kind)
        if not kind.endswith("-probe"):
            with (out / "logs" / f"tests-{kind}.log").open("wb") as log:
                subprocess.run(["cargo", "test", "--locked", "--release", "-p", "notist-core", "-p", "notist-pipeline", "-p", "notist", "--lib", "--tests", "--", "--test-threads=1"], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        m["binaries"][kind] = sha((out / "bin" / kind).read_bytes())
        save(out / "manifest.json", m)
        print("built and validated", kind, flush=True)


def revise_preparation(args):
    """Keep failed, never-measured preparation drafts before fixing builders."""
    out = args.out
    assert not (out / "samples.json").exists()
    m = json.loads((out / "manifest.json").read_text())
    drafts = out / "preparation-drafts"
    assert not drafts.exists()
    drafts.mkdir()
    save(drafts / "manifest.json", m)
    for kind in m["source_hashes"]:
        if kind in m["binaries"]:
            continue
        source = out / "sources" / kind
        source.rename(drafts / kind)
        original = out / "sources" / kind.removesuffix("-probe") if kind.endswith("-probe") else out / "sources/base"
        shutil.copytree(original, source)
        if kind.endswith("-probe"):
            observer(source)
        else:
            variant(source, kind)
        (out / "patches" / f"{kind}.patch").write_text(patch_between(original, source))
        m["source_hashes"][kind] = inventory(source)
    for file in (out / "reproducer").iterdir():
        shutil.copy2(REPO / "tools" / file.name, file)
    m["reproducer"] = inventory(out / "reproducer")
    save(out / "manifest.json", m)


def parse_observer(text):
    metrics, pages = {}, {}
    for line in text.splitlines():
        f = line.split()
        if line.startswith("[eval-pages] "):
            pages[f[1]] = {"seen": int(f[2]), "selected": int(f[3])}
        elif line.startswith("[eval-observe] "):
            metrics.setdefault(f[1], {})[f[2]] = dict(zip(["calls", "inclusive_ns", "self_ns", "allocs", "allocated_bytes", "frees", "freed_bytes", "reallocs"], map(int, f[3:])))
    assert sum(v["seen"] for v in pages.values()) == 51739
    for values in metrics.values():
        assert sum(m["self_ns"] for m in values.values()) == values["run"]["inclusive_ns"]
    return {"pages": pages, "metrics": metrics}


def process(args, kind, label, extra=None, rss=False):
    out = args.out
    env = dict(os.environ, NO_COLOR="1", TERM="dumb", PYTHONDONTWRITEBYTECODE="1", **(extra or {}))
    assert not os.environ.get("LD_PRELOAD"), "do not replace an existing interposer"
    for name in ["RESOURCE_PROFILE", "RESOURCE_DIAGNOSTIC", "XIV_PROFILE_REQUESTS"]:
        assert not env.get(name), name
    points, stop = [], threading.Event()
    start = time.perf_counter()
    with (out / "logs" / f"{label}.stdout").open("wb") as stdout, (out / "logs" / f"{label}.stderr").open("wb") as stderr:
        child = subprocess.Popen([str(out / "bin" / kind), "build"], cwd=out / "site", env=env, stdout=stdout, stderr=stderr)
        def sample():
            import re
            while not stop.wait(0.025):
                try:
                    match = re.search(r"VmRSS:\s+(\d+)", Path(f"/proc/{child.pid}/status").read_text())
                    if match:
                        points.append([time.perf_counter() - start, int(match[1])])
                except (OSError, ProcessLookupError):
                    pass
        if rss:
            thread = threading.Thread(target=sample)
            thread.start()
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
        wall = time.perf_counter() - start
        stop.set()
        if rss:
            thread.join()
    row = {"kind": kind, "label": label, "wall_s": wall, "user_s": usage.ru_utime, "system_s": usage.ru_stime, "exit": child.returncode,
           "major_faults": usage.ru_majflt, "minor_faults": usage.ru_minflt, "voluntary_context_switches": usage.ru_nvcsw, "involuntary_context_switches": usage.ru_nivcsw}
    assert child.returncode == 0, row
    text = (out / "logs" / f"{label}.stderr").read_text()
    if kind.endswith("-probe"):
        row["stages"], row["edges"], row["modules"] = parse_detail(text)
        row["observer"] = parse_observer(text)
    actual = inventory(out / "site/dist/icexiv-db")
    assert actual == json.loads((out / "output-inventory.json").read_text()), label
    row["output_verified"] = True
    if rss:
        row["peak_observed_rss_kib"] = max(point[1] for point in points)
        save(out / "logs" / f"{label}.rss.json", points)
    save(out / "logs" / f"{label}.json", row)
    print(label, round(wall, 4), flush=True)
    return row


def run(args):
    out = args.out
    assert not (out / "samples.json").exists()
    manifest = json.loads((out / "manifest.json").read_text())
    assert args.rounds == manifest["rounds"]
    assert {k: v for k, v in inventory(out / "site").items() if not k.startswith("dist/")} == manifest["inputs"]
    for kind in KINDS:
        assert inventory(out / "sources" / kind) == manifest["source_hashes"][kind]
        assert sha((out / "bin" / kind).read_bytes()) == manifest["binaries"][kind]
    samples = []
    for rep in range(-1, args.rounds):
        # Balanced Williams order: each treatment appears in every position and
        # each directed adjacent treatment pair occurs once per six rounds.
        kinds = [KINDS[(i + rep) % 6] for i in [0, 1, 5, 2, 4, 3]] if rep >= 0 else KINDS
        for kind in kinds:
            row = process(args, kind, f"normal-{rep}-{kind}")
            row["round"] = rep
            samples.append(row)
            save(out / "samples.json", samples)


def diagnose(args):
    rows = []
    for mode in ["off", "count", "time", "alloc"]:
        for kind in ["base-probe", "combined-probe"]:
            for rep in range(3 if mode == "time" else 1):
                row = process(args, kind, f"diagnostic-{mode}-{rep}-{kind}", {"EVAL_AB_OBSERVER": mode})
                row["mode"], row["round"] = mode, rep
                rows.append(row)
                save(args.out / "diagnostics.json", rows)
    for rep in range(2):
        for kind in KINDS if rep == 0 else KINDS[::-1]:
            row = process(args, kind, f"rss-{rep}-{kind}", rss=True)
            rows.append(row)
            save(args.out / "diagnostics.json", rows)


def heap(args):
    import re
    out = args.out
    assert not os.environ.get("LD_PRELOAD")
    assert not (out / "heap.json").exists()
    source, library = out / "native-alloc.c", out / "native-alloc.so"
    shutil.copy2(Path(__file__).with_name("evaluator_ab_alloc.c"), source)
    subprocess.run(["cc", "-shared", "-fPIC", "-O2", str(source), "-o", str(library)], check=True)
    rows = []
    for kind in KINDS:
        label = f"native-alloc-{kind}"
        row = process(args, kind, label, {"LD_PRELOAD": str(library), "EVAL_AB_ALLOC_TARGET": str(out / "bin" / kind)})
        text = (out / "logs" / f"{label}.stderr").read_text()
        match = re.findall(r"\[eval-native-alloc\] pid=(\d+) allocations=(\d+) requested_bytes=(\d+) frees=(\d+) reallocations=(\d+)", text)
        assert len(match) == 1, text
        row["api_events"] = dict(zip(["pid", "allocations", "requested_bytes", "frees", "reallocations"], map(int, match[0])))
        rows.append(row)
        save(out / "heap.json", {"source_sha256": sha(source.read_bytes()), "library_sha256": sha(library.read_bytes()), "samples": rows})


def semantic(args):
    """Compare complete public analysis/evaluation JSON, not just final HTML."""
    out = args.out
    fixture = out / "semantic-fixture"
    fixture.mkdir(exist_ok=True)
    cases = {
        "capture": 'fn view() -> InlineContent { let x = 9; let v = for x in (1,2) { for y in (3,4) [#text(text: str(x))#text(text: str(y))] }; [@(id: "one")#v#text(text: str(x))@(id: "two")#v] }',
        "failure": 'fn view() -> InlineContent { [#text(text: panic("inner failure")) kept] }',
        "generic": 'fn pass<T: Type>(value: T) -> T { value } fn view() -> InlineContent { [#text(text: str(pass<Int>(7)))] }',
        "data": 'fn view() -> InlineContent { let v = for x in data_expect<Array<Int>>(data_decode<Array<Int>>(data_expect<Data>(read_json(p"./values.json")))) [#text(text: str(x))]; [#v#v] }',
    }
    (fixture / "doc.not").write_text('#import "lib.notc" as views;\n#views.view()\n')
    (fixture / "values.json").write_text('[1,2,3]\n')
    result = {}
    for name, source in cases.items():
        (fixture / "lib.notc").write_text(source)
        expected = None
        for kind in KINDS:
            child = subprocess.run([str(out / "bin" / kind), "--eval", "json", "doc.not"], cwd=fixture, text=True, capture_output=True, check=True)
            actual = json.loads(child.stdout)
            assert "core" in actual, child.stdout
            if expected is None:
                expected = actual
            assert actual == expected, (name, kind)
            save(out / "logs" / f"semantic-{name}-{kind}.json", actual)
        result[name] = {"source": source, "variants": len(KINDS), "identical": True, "evaluation_diagnostics": expected.get("evaluation_diagnostics")}
    save(out / "semantic.json", result)
    print("complete semantic JSON matched", len(cases), "cases x", len(KINDS), "variants", flush=True)


def serve_ab(args):
    import re
    out, rows = args.out, []
    trigger = out / "site/content/index.not"
    original = trigger.read_bytes()
    for kind in ["base", "combined"]:
        log = out / "logs" / f"serve-{kind}.stderr"
        with log.open("wb") as stderr:
            child = subprocess.Popen([str(out / "bin" / kind), "serve", "--port", "0", "--no-clear-screen"], cwd=out / "site", env=dict(os.environ, TERM="dumb", NO_COLOR="1", PYTHONDONTWRITEBYTECODE="1"), stdout=subprocess.DEVNULL, stderr=stderr)
            seen = 0
            deadline = time.monotonic() + 600
            try:
                while seen < 4:
                    assert child.poll() is None, log.read_text()
                    matches = re.findall(r"Updated · (\d+) pages · (\d+)ms", log.read_text())
                    if len(matches) > seen:
                        pages, elapsed = map(int, matches[seen])
                        assert pages == 51739
                        rows.append({"kind": kind, "round": seen, "pages": pages, "elapsed_ms": elapsed})
                        print("serve", rows[-1], flush=True)
                        save(out / "serve.json", rows)
                        seen += 1
                        if seen < 4:
                            time.sleep(2)
                            trigger.write_bytes(original)
                            deadline = time.monotonic() + 600
                    assert time.monotonic() < deadline, log.read_text()
                    time.sleep(0.2)
            finally:
                child.terminate()
                child.wait(timeout=30)
    assert trigger.read_bytes() == original


def report(args):
    out = args.out
    m = json.loads((out / "manifest.json").read_text())
    assert inventory(out / "reproducer") == m["reproducer"]
    for kind, expected in m["source_hashes"].items():
        assert inventory(out / "sources" / kind) == expected
        assert sha((out / "bin" / kind).read_bytes()) == m["binaries"][kind]
    assert {k: v for k, v in inventory(out / "site").items() if not k.startswith("dist/")} == m["inputs"]
    samples = json.loads((out / "samples.json").read_text())
    base = {r["round"]: r for r in samples if r["kind"] == "base" and r["round"] >= 0}
    result = {}
    for kind in KINDS:
        rows = [r for r in samples if r["kind"] == kind and r["round"] >= 0]
        paired = [100 * (base[r["round"]]["wall_s"] - r["wall_s"]) / base[r["round"]]["wall_s"] for r in rows]
        rng = random.Random(20261104)
        boot = sorted(statistics.median(rng.choices(paired, k=len(paired))) for _ in range(10000))
        result[kind] = {"wall_median": statistics.median(r["wall_s"] for r in rows), "wall_min": min(r["wall_s"] for r in rows), "wall_max": max(r["wall_s"] for r in rows), "user_median": statistics.median(r["user_s"] for r in rows), "system_median": statistics.median(r["system_s"] for r in rows), "paired_gain_percent": paired, "paired_median_gain_percent": statistics.median(paired), "paired_bootstrap_median_95_percent": [boot[250], boot[9749]], "faster_rounds": sum(p > 0 for p in paired)}
    save(out / "summary.json", result)
    print(json.dumps(result, indent=2))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("action", choices=["prepare", "build", "revise_preparation", "run", "diagnose", "heap", "semantic", "serve_ab", "report"])
    p.add_argument("--out", required=True, type=Path)
    p.add_argument("--base", type=Path, default=Path("/tmp/opencode/notist-icexiv-profile-20261104"))
    p.add_argument("--rounds", type=int, default=6)
    args = p.parse_args()
    args.out, args.base = args.out.resolve(), args.base.resolve()
    globals()[args.action](args)


if __name__ == "__main__":
    main()
