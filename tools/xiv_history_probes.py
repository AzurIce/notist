"""Disposable instrumentation and measurements for profile_xiv_history.py.

All source rewriting is restricted to copies under --out. Instrumented timings
are inclusive, and never substituted for normal-binary CLI timings.
"""
import csv
import difflib
from html.parser import HTMLParser
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import time

from profile_xiv_history import CASES, ENGINES, once, save, sha


SINK = r'''//! Temporary, std-only inclusive stage probes; not production code.
use std::{cell::RefCell, collections::BTreeMap, time::Instant};
thread_local! {
    static TOTALS: RefCell<BTreeMap<&'static str, (u64, u128)>> = RefCell::default();
}
pub struct Span { name: &'static str, start: Instant }
pub fn span(name: &'static str) -> Span { Span { name, start: Instant::now() } }
impl Drop for Span {
    fn drop(&mut self) {
        let nanos = self.start.elapsed().as_nanos();
        TOTALS.with(|cell| {
            let mut totals = cell.borrow_mut();
            let entry = totals.entry(self.name).or_default();
            entry.0 += 1;
            entry.1 += nanos;
        });
    }
}
pub fn measure<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let _span = span(name);
    f()
}
pub fn take() -> BTreeMap<&'static str, (u64, u128)> {
    TOTALS.with(|cell| std::mem::take(&mut *cell.borrow_mut()))
}
pub struct Report;
impl Drop for Report {
    fn drop(&mut self) {
        for (name, (calls, nanos)) in take() {
            eprintln!("[xiv-stage] {name} {calls} {:.9}", nanos as f64 / 1e9);
        }
        __MODULE_STATS__
    }
}
'''

MODULE_STATS = r'''let stats = crate::profiling::take();
        for (name, m) in [
            ("compile", stats.compile), ("compile_work", stats.compile_work),
            ("bind", stats.bind), ("bind_work", stats.bind_work),
            ("link", stats.link), ("install", stats.install),
        ] {
            eprintln!("[xiv-module] {name} {} {:.9}", m.calls, m.nanos as f64 / 1e9);
        }'''


def enter_function(text, name, lines):
    matches = list(re.finditer(r"\bfn " + re.escape(name) + r"(?:<[^\n]*>)?\(", text))
    if len(matches) != 1:
        raise ValueError(f"expected one function {name}, got {len(matches)}")
    opening = text.index("{", matches[0].end()) + 1
    return text[:opening] + "\n" + lines + text[opening:]


def measure_rhs(text, binding, end, name, prefix="notist::xiv_profile"):
    start = text.index(binding) + len(binding)
    finish = text.index(end, start)
    expression = text[start:finish]
    fallible = expression.endswith("?")
    if fallible:
        expression = expression[:-1]
    return (text[:start] + f'{prefix}::measure("{name}", || ' +
            expression + ")" + ("?" if fallible else "") + text[finish:])


def instrument(source, engine):
    root = source / "src/lib.rs"
    root.write_text(root.read_text() + "\n#[doc(hidden)]\npub mod xiv_profile;\n")
    has_stats = (source / "crates/notist-pipeline/src/profiling.rs").exists()
    (source / "src/xiv_profile.rs").write_text(SINK.replace("__MODULE_STATS__", MODULE_STATS if has_stats else ""))
    path = source / "crates/notist-ssg/src/build.rs"
    text = enter_function(path.read_text(), "build", '''    let _xiv_total = notist::xiv_profile::span("ssg.total");
    let xiv_phase = notist::xiv_profile::span("ssg.discover");''')
    transitions = [
        ("    crate::extensions::prepare(vault, config, extensions, &mut input)?;", "ssg.prepare", False),
        ("    let mut source_dirs: BTreeSet<_> =", "ssg.source_directories", False),
        ("    let mut documents = vec![];", "ssg.pipeline", False),
        ("    let mut assets = Assets::new(output);", "ssg.assets_templates", False),
        ("    let by_source: BTreeMap<_, _> =", "ssg.bodies", False),
        ("    let dirs = directories(&pages, &source_dirs, &content);", "ssg.shared_context", False),
        ("    let mut current_directory: Option<&str> = None;", "ssg.page_templates", False),
    ]
    for anchor, stage, _ in transitions:
        text = once(text, anchor, f'    drop(xiv_phase);\n    let xiv_phase = notist::xiv_profile::span("{stage}");\n' + anchor)
    # Start finish after the final page loop, before input validation if present.
    anchor = "    vault.validate_inputs()?;\n    Ok(BuildOutput {" if "    vault.validate_inputs()?;\n    Ok(BuildOutput {" in text else "    Ok(BuildOutput {"
    text = once(text, anchor, '    drop(xiv_phase);\n    let xiv_phase = notist::xiv_profile::span("ssg.finish");\n' + anchor)
    text = once(text, "        files: assets.finish()?,", '        files: notist::xiv_profile::measure("finish.assets", || assets.finish())?,')
    # Explicitly drop the finish span before the return's large local teardown.
    text = once(text, "    Ok(BuildOutput {", "    let xiv_result = Ok(BuildOutput {")
    text = once(text, "        warnings,\n    })\n}", "        warnings,\n    });\n    drop(xiv_phase);\n    xiv_result\n}")
    for binding, end, stage in [
        ("let analysis = ", ";\n        errors.extend(", "pipeline.analyze"),
        ("let evaluation = ", ";\n        errors.extend(", "pipeline.evaluate"),
        ("let rendered = ", ";\n        errors.extend(rendered", "body.html"),
        ("let registrations = ", ";\n        if !registrations", "body.components"),
        ("let html = ", ";\n        assets.generated(page", "template.render"),
    ]:
        text = measure_rhs(text, binding, end, stage)
    binding = "let (registry, environment) = " if "let (registry, environment) = " in text else "let registry = "
    text = measure_rhs(text, binding, ";\n        let rendered", "body.registry")
    if "let environment = vault.environment_for(" in text:
        text = measure_rhs(text, "let environment = ", ";\n        let component_path", "body.environment")
    text = once(text, "        let mut resolved = BTreeMap::new();", '        let xiv_links = notist::xiv_profile::span("body.links");\n        let mut resolved = BTreeMap::new();')
    text = once(text, "        " + binding, "        drop(xiv_links);\n        " + binding)
    text = once(text, "        let template = page\n", '        let xiv_context = notist::xiv_profile::span("template.context");\n        let template = page\n')
    text = once(text, "        let html = ", "        drop(xiv_context);\n        let html = ")
    text = once(text, "        assets.generated(page.route.output.clone(), html.into_bytes())?;",
                '        notist::xiv_profile::measure("template.output", || assets.generated(page.route.output.clone(), html.into_bytes()))?;')
    path.write_text(text)
    path = source / "src/vault.rs"
    text = path.read_text()
    for name in ["ensure_environment", "environment_key", "environment_snapshot", "environment_for", "compile", "evaluate_in"]:
        if re.search(r"\bfn " + name + r"\(", text):
            text = enter_function(text, name, f'        let _xiv = crate::xiv_profile::span("vault.{name}");')
    # Document lowering is separate from notc module compilation and environments.
    text = measure_rhs(text, "let output = ", ";\n        let ", "vault.document_lower", "crate::xiv_profile")
    text = measure_rhs(text, "let (mut analysis, inspection) =", ";\n        analysis.extend", "vault.document_analysis", "crate::xiv_profile")
    path.write_text(text)
    path = source / "src/stdio.rs"
    text = path.read_text()
    text = enter_function(text, "exchange", '    let _xiv = crate::xiv_profile::span("stdio.total");')
    text = once(text, "    let input = serde_json::to_vec(request)", '    let xiv_phase = crate::xiv_profile::span("stdio.encode");\n    let input = serde_json::to_vec(request)')
    text = once(text, "    let mut child = Command::new(program)", '    drop(xiv_phase);\n    let xiv_phase = crate::xiv_profile::span("stdio.process");\n    let mut child = Command::new(program)')
    text = once(text, "    serde_json::from_slice(&output.stdout)", '    drop(xiv_phase);\n    let _xiv_decode = crate::xiv_profile::span("stdio.decode");\n    serde_json::from_slice(&output.stdout)')
    path.write_text(text)
    path = source / "crates/notist-cli/src/ssg.rs"
    text = enter_function(path.read_text(), "assemble", '    let _xiv = notist::xiv_profile::span("cli.assemble");')
    text = enter_function(text, "build_target", '    let _xiv = notist::xiv_profile::span("cli.build_target");')
    path.write_text(text)
    path = source / "crates/notist-cli/src/publication.rs"
    text = enter_function(path.read_text(), "publish", '    let _xiv = notist::xiv_profile::span("cli.publish");')
    text = once(text, "    for (relative, bytes) in files {", '    let xiv_write = notist::xiv_profile::span("publish.write");\n    for (relative, bytes) in files {')
    text = once(text, '    let backup = tempfile::Builder::new()', '    drop(xiv_write);\n    let backup = tempfile::Builder::new()')
    path.write_text(text)
    path = source / "crates/notist-cli/src/main.rs"
    lines = '''    let _xiv_report = notist::xiv_profile::Report;
    let _xiv_total = notist::xiv_profile::span("cli.total");'''
    if has_stats:
        mode = {"memo-off": "Baseline", "memo-compile": "CompileOnly"}.get(engine, "Both")
        lines += f"\n    notist::profiling::set_cache_mode(notist::profiling::CacheMode::{mode});"
    path.write_text(enter_function(path.read_text(), "main", lines))
    example = source / "crates/notist-cli/examples/module_cache_profile.rs"
    if example.exists():
        text = example.read_text()
        text = once(text, "    let stats = profiling::take();", "    let history_stages = notist::xiv_profile::take();\n    let stats = profiling::take();")
        text = once(text, '"pages": pages, "files": files, "bytes": bytes, "stages": stages', '"pages": pages, "files": files, "bytes": bytes, "stages": stages, "history_stages": history_stages')
        text = text.replace("        profiling::take();", "        notist::xiv_profile::take();\n        profiling::take();")
        example.write_text(text)


def profile_build(args):
    out = args.out.resolve()
    manifest = json.loads((out / "manifest.json").read_text())
    for name, _, _ in ENGINES:
        original = out / "sources" / name
        source = out / "profile-sources" / name
        binary = out / "bin" / f"profile-{name}"
        if binary.exists():
            continue
        if not source.exists():
            shutil.copytree(original, source)
            instrument(source, name)
            patches = []
            for p in sorted(source.rglob("*.rs")):
                rel = p.relative_to(source)
                a = (original / rel).read_text() if (original / rel).is_file() else ""
                b = p.read_text()
                if a != b:
                    patches.extend(difflib.unified_diff(a.splitlines(True), b.splitlines(True), fromfile="a/" + str(rel), tofile="b/" + str(rel)))
            (out / "logs" / f"probes-{name}.patch").write_text("".join(patches))
        target = out / "targets" / name
        env = dict(os.environ, CARGO_TARGET_DIR=str(target), **manifest["build_env"], NO_COLOR="1", TERM="dumb")
        # Archive mtimes cannot be trusted for cross-directory Cargo freshness.
        # Remove only this disposable target's local workspace artifacts, keeping
        # third-party dependencies; the normal binaries are already copied out.
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=source, env=env))
        packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
        with (out / "logs" / f"profile-build-{name}.log").open("wb") as log:
            subprocess.run(["cargo", "clean", "--release", *[flag for p in packages for flag in ["-p", p]]], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            command = ["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist"]
            if (source / "crates/notist-pipeline/src/profiling.rs").exists():
                command += ["--features", "profiling"]
            subprocess.run(command, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            shutil.copy2(target / "release/notist", binary)
            if name == "memo-both":
                subprocess.run(["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--features", "profiling", "--example", "module_cache_profile"], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
                shutil.copy2(target / "release/examples/module_cache_profile", out / "bin/module-rounds")
        # Keep profile provenance separate: the normal builder owns manifest.json.
        save(out / "logs" / f"profile-binary-{name}.json", {"sha256": sha(binary.read_bytes()), "source": str(source)})
        print(f"profile built {name}", flush=True)


def configure(out, case):
    name, engine, script, _ = case
    dest = out / "site/templates"
    if dest.exists():
        # Only our copied, known template directory is replaced.
        shutil.rmtree(dest)
    shutil.copytree(out / ("templates-handlebars" if engine == "handlebars" else "templates-tera"), dest)
    shutil.copy2(out / "scripts" / f"{script}.py", out / "site/scripts/items.py")


def process(command, out, label, cpu, requests=True, check=True):
    env = dict(os.environ, NO_COLOR="1", TERM="dumb")
    env.pop("XIV_PROFILE_REQUESTS", None)
    if requests:
        env["XIV_PROFILE_REQUESTS"] = "1000000"
    command = ["taskset", "-c", str(cpu), *map(str, command)]
    log = out / "logs" / label
    started = time.perf_counter()
    with log.with_suffix(".stdout").open("wb") as stdout, log.with_suffix(".stderr").open("wb") as stderr:
        child = subprocess.Popen(command, cwd=out / "site", env=env, stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    result = {"command": command, "wall_s": time.perf_counter() - started,
              "user_s": usage.ru_utime, "system_s": usage.ru_stime,
              "peak_rss_kib": usage.ru_maxrss, "returncode": child.returncode,
              "stdout": str(log.with_suffix(".stdout")), "stderr": str(log.with_suffix(".stderr"))}
    if check and child.returncode:
        raise RuntimeError(str(result) + "\n" + log.with_suffix(".stderr").read_text()[-5000:])
    return result


def fixed_inputs(out):
    manifest = json.loads((out / "manifest.json").read_text())
    changed = [rel for rel, expected in manifest["fixed_site_files"].items()
               if not (out / "site" / rel).is_file() or sha((out / "site" / rel).read_bytes()) != expected]
    assert not changed, f"fixed site input changed: {changed}"


class SemanticHTML(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.tokens = []

    def handle_decl(self, decl):
        self.tokens.append(["decl", decl])

    def handle_starttag(self, tag, attrs):
        self.tokens.append(["start", tag, sorted(attrs)])

    def handle_startendtag(self, tag, attrs):
        self.tokens.append(["empty", tag, sorted(attrs)])

    def handle_endtag(self, tag):
        self.tokens.append(["end", tag])

    def handle_data(self, text):
        if text.strip():
            self.tokens.append(["text", text])

    def handle_comment(self, text):
        self.tokens.append(["comment", text])


def inventory(root, semantic=False):
    paths, semantics = {}, {}
    size = html = 0
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        rel = path.relative_to(root).as_posix()
        raw = path.read_bytes()
        paths[rel] = sha(raw)
        size += len(raw)
        if path.suffix == ".html":
            html += 1
            if semantic:
                parser = SemanticHTML()
                parser.feed(raw.decode())
                parser.close()
                semantics[rel] = sha(json.dumps(parser.tokens, ensure_ascii=False, separators=(",", ":")).encode())
        elif semantic:
            semantics[rel] = paths[rel]
    digest = hashlib.sha256()
    for rel, value in paths.items():
        digest.update(rel.encode() + b"\0" + bytes.fromhex(value))
    return {"files": len(paths), "html": html, "bytes": size, "sha256": digest.hexdigest(),
            "paths": paths, "semantics": semantics}


def check_output(out, cpu, label):
    result = process(["python3", "scripts/check_site.py", "dist/xiv-db"], out, f"verify-{label}", cpu)
    return {"wall_s": result["wall_s"], "stdout": Path(result["stdout"]).read_text().strip()}


def run(args):
    out = args.out.resolve()
    fixed_inputs(out)
    # A failed/partial build must never silently reuse the preliminary binaries.
    manifest = json.loads((out / "manifest.json").read_text())
    for engine, _, _ in ENGINES:
        if manifest["engines"][engine].get("target_dir") != str(out / "targets" / engine):
            raise ValueError(f"independent build missing: {engine}")
        if sha((out / "bin" / engine).read_bytes()) != manifest["engines"][engine]["binary_sha256"]:
            raise ValueError(f"binary fingerprint mismatch: {engine}")
    result_path = out / "runs.json"
    result = json.loads(result_path.read_text()) if result_path.exists() else {"warmups": [], "samples": [], "validations": {}, "default_budget": []}
    if result["samples"] and (result["method"]["runs"] != args.runs or result["method"]["cpu"] != args.cpu):
        raise ValueError("cannot resume with different round count or CPU; use a new result directory")
    result["method"] = {"runs": args.runs, "cpu": args.cpu, "requests": 1000000,
                        "system": platform.platform(), "python": platform.python_version(),
                        "clock": "perf_counter", "rss": "wait4 ru_maxrss KiB, not simultaneous process-tree sum",
                        "cache": "warm filesystem, fresh process for each normal CLI sample", "fsync": False}
    cpu_info = Path(f"/sys/devices/system/cpu/cpu{args.cpu}/cpufreq")
    result["method"]["cpu_frequency_policy"] = {p.name: p.read_text().strip() for p in cpu_info.glob("scaling_*")
                                                 if p.name in ["scaling_governor", "scaling_driver", "scaling_min_freq", "scaling_max_freq"]}
    result["method"]["cpu_model"] = next(line.split(":", 1)[1].strip() for line in Path("/proc/cpuinfo").read_text().splitlines() if line.startswith("model name"))
    baseline = None
    semantics = None
    for case in CASES:
        name, engine, _, _ = case
        configure(out, case)
        file = out / "inventories" / f"{name}.json"
        if name not in result["validations"]:
            sample = process([out / "bin" / engine, "build"], out, f"warmup-{name}", args.cpu)
            sample["case"] = name
            result["warmups"].append(sample)
            # Only Handlebars and first Tera need parsing for cross-engine
            # semantics; all remaining comparisons require exact byte equality.
            current = inventory(out / "site/dist/xiv-db", semantic=name in ["handlebars", "tera"])
            save(file, current)
            assert current["files"] == 51744 and current["html"] == 51739, current
            result["validations"][name] = check_output(out, args.cpu, name)
            result["validations"][name].update({k: v for k, v in current.items() if k not in ["paths", "semantics"]})
            save(result_path, result)
        else:
            current = json.loads(file.read_text())
        if name == "handlebars":
            semantics = current["semantics"]
        elif name == "tera":
            assert current["semantics"] == semantics, "Handlebars/Tera HTML semantics or non-HTML bytes differ"
            baseline = current["paths"]
        else:
            different = [p for p in current["paths"].keys() | baseline.keys() if current["paths"].get(p) != baseline.get(p)]
            assert not different, f"{name}: byte differences: {different[:20]}"
        print(f"validated {name}: {current['files']} files", flush=True)
    # Rotate and reverse, so one implementation is not always measured first.
    for round_no in range(args.runs):
        shift = (round_no * 5) % len(CASES)
        order = CASES[shift:] + CASES[:shift]
        if round_no % 2:
            order = list(reversed(order))
        for case in order:
            name, engine, _, _ = case
            if any(s["case"] == name and s["round"] == round_no for s in result["samples"]):
                continue
            configure(out, case)
            sample = process([out / "bin" / engine, "build"], out, f"cli-{round_no}-{name}", args.cpu)
            sample.update(case=name, round=round_no)
            result["samples"].append(sample)
            save(result_path, result)
            print(f"CLI {round_no + 1}/{args.runs} {name}: {sample['wall_s']:.4f}s", flush=True)
    for case in CASES:
        name, engine, _, _ = case
        if engine not in ["resources", "modules", "listen", "memo-off", "memo-compile", "memo-both", "head", "working"]:
            continue
        if any(s["case"] == name for s in result["default_budget"]):
            continue
        configure(out, case)
        sample = process([out / "bin" / engine, "build"], out, f"default-budget-{name}", args.cpu, requests=False, check=False)
        sample["case"] = name
        assert sample["returncode"] != 0, "default budget unexpectedly succeeds; inspect rather than assuming"
        result["default_budget"].append(sample)
        save(result_path, result)
    fixed_inputs(out)


def parse_stages(path):
    stages, modules = {}, {}
    for line in path.read_text().splitlines():
        if line.startswith(("[xiv-stage] ", "[xiv-module] ")):
            tag, name, calls, seconds = line.split()
            (stages if tag == "[xiv-stage]" else modules)[name] = {"calls": int(calls), "seconds": float(seconds)}
    assert stages, path
    return stages, modules


def profiles(args):
    out = args.out.resolve()
    fixed_inputs(out)
    file = out / "profiles.json"
    result = json.loads(file.read_text()) if file.exists() else {"samples": [], "module_rounds": []}
    # Three rounds are enough for inclusive attribution; CLI rankings use five
    # rounds of completely uninstrumented binaries, not these probes.
    cases = [case for case in CASES if case[1] != "handlebars"]
    for round_no in range(3):
        order = cases[round_no * 5:] + cases[:round_no * 5]
        if round_no % 2:
            order = list(reversed(order))
        for case in order:
            name, engine, _, _ = case
            if any(s["case"] == name and s["round"] == round_no for s in result["samples"]):
                continue
            configure(out, case)
            sample = process([out / "bin" / f"profile-{engine}", "build"], out, f"stages-{round_no}-{name}", args.cpu)
            sample.update(case=name, round=round_no)
            sample["stages"], sample["modules"] = parse_stages(Path(sample["stderr"]))
            result["samples"].append(sample)
            save(file, result)
            if round_no == 0:
                current = inventory(out / "site/dist/xiv-db")
                expected = json.loads((out / "inventories" / f"{name}.json").read_text())
                assert current["paths"] == expected["paths"], f"probe changed output: {name}"
            print(f"stages {round_no + 1}/3 {name}", flush=True)
    configure(out, CASES[-1])
    # One binary with profiling-only mode controls avoids codegen differences in
    # the three same-process cache tests. It still runs the actual complete site.
    for rep in range(3):
        for mode in (["baseline", "compile", "both"] if rep % 2 == 0 else ["both", "compile", "baseline"]):
            if any(s["mode"] == mode and s["rep"] == rep for s in result["module_rounds"]):
                continue
            sample = process([out / "bin/module-rounds", "site", out / "site", "3", out / "module-round-output", mode], out, f"module-rounds-{rep}-{mode}", args.cpu)
            sample.update(mode=mode, rep=rep)
            sample["rounds"] = [json.loads(line) for line in Path(sample["stdout"]).read_text().splitlines()]
            assert all(r["pages"] == 51739 and r["files"] == 51744 for r in sample["rounds"])
            result["module_rounds"].append(sample)
            save(file, result)
            print(f"module rounds {rep + 1}/3 {mode}", flush=True)
    current = inventory(out / "module-round-output")
    expected = json.loads((out / "inventories/working.json").read_text())
    assert current["paths"] == expected["paths"], "same-process module output changed"
    result["module_output_validation"] = {k: current[k] for k in ["files", "html", "bytes", "sha256"]}
    save(file, result)
    fixed_inputs(out)


def distribution(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values), "values": values}


def report(args):
    out = args.out.resolve()
    runs = json.loads((out / "runs.json").read_text())
    probes = json.loads((out / "profiles.json").read_text())
    manifest = json.loads((out / "manifest.json").read_text())
    summary = {"cases": [], "module_rounds": {}, "method": runs["method"]}
    for name, engine, script, label in CASES:
        normal = [s for s in runs["samples"] if s["case"] == name]
        samples = [s for s in probes["samples"] if s["case"] == name]
        row = {"case": name, "label": label, "revision": manifest["engines"][engine]["revision"], "script": script,
               "cli": {key: distribution([s[key] for s in normal]) for key in ["wall_s", "user_s", "system_s", "peak_rss_kib"]},
               "stages": {}, "modules": {}, "validation": runs["validations"][name]}
        for kind in ["stages", "modules"]:
            for key in sorted({k for s in samples for k in s[kind]}):
                row[kind][key] = {field: distribution([s[kind].get(key, {}).get(field, 0) for s in samples]) for field in ["calls", "seconds"]}
        if samples:
            row["probe_cli_wall_s"] = distribution([s["wall_s"] for s in samples])
            stage_names = ["discover", "prepare", "source_directories", "pipeline", "assets_templates", "bodies", "shared_context", "page_templates", "finish"]
            residuals = {
                "ssg.local_release": [s["stages"]["ssg.total"]["seconds"] - sum(s["stages"]["ssg." + n]["seconds"] for n in stage_names) for s in samples],
                "cli.prepare_vault_release": [s["stages"]["cli.assemble"]["seconds"] - s["stages"]["ssg.total"]["seconds"] for s in samples],
                "publish.other_cleanup": [s["stages"]["cli.publish"]["seconds"] - s["stages"]["publish.write"]["seconds"] for s in samples],
            }
            row["residuals"] = {name: distribution(values) for name, values in residuals.items()}
        summary["cases"].append(row)
    for mode in ["baseline", "compile", "both"]:
        items = [s for s in probes["module_rounds"] if s["mode"] == mode]
        first = [s["rounds"][0]["wall_ms"] for s in items]
        later = [r["wall_ms"] for s in items for r in s["rounds"][1:]]
        summary["module_rounds"][mode] = {"first_ms": distribution(first), "later_ms": distribution(later), "runs": items}
    save(out / "summary.json", summary)
    with (out / "summary.csv").open("w", newline="") as file:
        writer = csv.writer(file)
        writer.writerow(["case", "label", "revision", "script", "wall_median_s", "wall_min_s", "wall_max_s", "rss_median_mib", "user_median_s", "sys_median_s"])
        for row in summary["cases"]:
            cli = row["cli"]
            writer.writerow([row["case"], row["label"], row["revision"], row["script"], cli["wall_s"]["median"], cli["wall_s"]["min"], cli["wall_s"]["max"], cli["peak_rss_kib"]["median"] / 1024, cli["user_s"]["median"], cli["system_s"]["median"]])
    write_report(out, summary, runs, manifest)
    print(f"wrote {out / 'report.md'}", flush=True)


def write_report(out, summary, runs, manifest):
    rows = summary["cases"]
    by_name = {r["case"]: r for r in rows}
    first, tera, current = rows[0], by_name["tera"], by_name["working"]
    wall = lambda r: r["cli"]["wall_s"]["median"]
    rss = lambda r: r["cli"]["peak_rss_kib"]["median"] / 1024
    stage = lambda r, n: r["stages"].get(n, {}).get("seconds", {}).get("median", 0) * 1000
    reduction = lambda a, b: (1 - a / b) * 100
    env, res, semantic, catalog = [by_name[n] for n in ["environment", "resources", "semantic", "catalog"]]
    overhead = wall(res) - wall(env)
    lines = [
        "# XIV DB 全历程性能对照（2026-11-02）", "",
        "## 结论", "",
        f"- 同批完整生产站点：Tera 迁移基线 **{wall(tera):.3f} s → 当前工作区 {wall(current):.3f} s**，减少 **{reduction(wall(current), wall(tera)):.1f}%**，速度 **{wall(tera) / wall(current):.2f}×**。",
        f"- 峰值 RSS **{rss(tera):.1f} → {rss(current):.1f} MiB**，减少 **{reduction(rss(current), rss(tera)):.1f}%**。",
        f"- 存档 Handlebars 补充参照为 **{wall(first):.3f} s / {rss(first):.1f} MiB**；完整未提交源码未恢复，不与可重建的 Tera 主线混淆。",
        "- 全量为 51,225 详情 + 513 目录 + 首页，共 51,739 HTML、51,744 文件。没有删物品或改为客户端生成正文。", "",
        "### 收益与退化", "",
        f"- 最大的单步耗时收益来自去除整站 JSON 中间树（−{wall(by_name['html-shared']) - wall(by_name['borrowed']):.3f} s）、Python 批量编码（−{wall(by_name['context']) - wall(by_name['json-batch']):.3f} s）及共享语义定义（−{wall(catalog) - wall(semantic):.3f} s）。Renderer 直接构造与共享 HTML targets 两步合计减少约 {wall(tera) - wall(by_name['html-shared']):.3f} s。",
        f"- 共享语义定义同时将 RSS 从 {rss(catalog):.1f} 降至 {rss(semantic):.1f} MiB，减少约 {rss(catalog) - rss(semantic):.1f} MiB；共享页面／目录值另减少约 {rss(by_name['borrowed']) - rss(by_name['context']):.1f} MiB。",
        f"- 环境句柄优化版为 {wall(env):.3f} s；资源／共享值／Data 重构后为 {wall(res):.3f} s，增加 {overhead:.3f} s（{overhead / wall(env) * 100:.1f}%）及约 {rss(res) - rss(env):.1f} MiB RSS。对应 profile 的分析＋求值管线从 {stage(env, 'ssg.pipeline'):.0f} 增至 {stage(res, 'ssg.pipeline'):.0f} ms（+{stage(res, 'ssg.pipeline') - stage(env, 'ssg.pipeline'):.0f} ms），prepare 与模板阶段基本不变；其中 analyze 约 +{stage(res, 'pipeline.analyze') - stage(env, 'pipeline.analyze'):.0f} ms、evaluate 约 +{stage(res, 'pipeline.evaluate') - stage(env, 'pipeline.evaluate'):.0f} ms，包含式子阶段不可再次累加。这里定位到阶段，不把整项差异归因于某一个资源检查函数。",
        f"- 当前 {wall(current):.3f} s 仍比环境句柄优化版慢约 {(wall(current) / wall(env) - 1) * 100:.1f}%。资源重构以后的模块分层、comemo、移除 Markdown helper 及未提交语言能力，profile 主要阶段接近；小幅中位数差异和 CLI listen 控制点的浮动不作独立优化或回退结论。",
        f"- 当前主要成本是分析／求值约 {stage(current, 'ssg.pipeline') / 1000:.3f} s、发布约 {stage(current, 'cli.publish') / 1000:.3f} s、prepare 约 {stage(current, 'ssg.prepare') / 1000:.3f} s。纯 Tera render 仅约 {stage(current, 'template.render') / 1000:.3f} s（逐页模板管线合计约 {stage(current, 'ssg.page_templates') / 1000:.3f} s），正文 HTML renderer 约 {stage(current, 'body.html') / 1000:.3f} s，已不是主要瓶颈。发布中写文件约 {stage(current, 'publish.write') / 1000:.3f} s，安全检查／交换／旧目录清理等余项约 {current['residuals']['publish.other_cleanup']['median']:.3f} s。", "",
        "## 1. 同批普通 CLI 对照", "",
        f"每个点预热并验证；各 {runs['method']['runs']} 次新进程，轮转并正反序交错，CPU {runs['method']['cpu']}。表中为中位数及实测范围。off/compile 是同一 comemo 提交的消融，不是新的历史提交。", "",
        "| 对照点 | 引擎 | 脚本 | wall s（范围） | RSS MiB | user + sys s |",
        "| --- | --- | --- | ---: | ---: | ---: |",
    ]
    for r in rows:
        rev = r["revision"][:7] if r["case"] != "handlebars" else "存档"
        cli = r["cli"]
        lines.append(f"| {r['case']} · {r['label']} | {rev} | {r['script']} | {wall(r):.4f} ({cli['wall_s']['min']:.4f}–{cli['wall_s']['max']:.4f}) | {rss(r):.1f} | {cli['user_s']['median']:.3f} + {cli['system_s']['median']:.3f} |")
    lines += ["", "## 2. 阶段归因", "",
              "另外构建了插桩二进制，每点三轮。下表为包含式 ms 中位数，不替代上表无插桩 CLI。父阶段包含其详细子阶段，不可再与子阶段相加；不同列的中位数也不保证严格可加。", "",
              "| 对照点 | prepare | 分析＋求值管线 | 正文管线 | 共享模板上下文 | 逐页模板管线 | SSG 局部释放残差 | 发布写盘＋旧目录清理 |",
              "| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |"]
    for r in rows[1:]:
        values = [stage(r, n) for n in ["ssg.prepare", "ssg.pipeline", "ssg.bodies", "ssg.shared_context", "ssg.page_templates"]]
        values += [r["residuals"]["ssg.local_release"]["median"] * 1000, stage(r, "cli.publish")]
        lines.append("| " + r["case"] + " | " + " | ".join(f"{v:.2f}" for v in values) + " |")
    lines += ["", "### 当前版本详细阶段", "",
              "| 阶段（包含式） | 调用次数 | ms |", "| --- | ---: | ---: |"]
    for name in ["cli.total", "cli.assemble", "ssg.total", "ssg.prepare", "stdio.process", "stdio.decode", "ssg.pipeline", "pipeline.analyze", "vault.document_lower", "vault.ensure_environment", "vault.document_analysis", "pipeline.evaluate", "ssg.bodies", "body.registry", "body.html", "body.components", "ssg.shared_context", "ssg.page_templates", "template.context", "template.render", "template.output", "cli.publish", "publish.write"]:
        item = current["stages"].get(name)
        if item:
            lines.append(f"| {name} | {item['calls']['median']:.0f} | {item['seconds']['median'] * 1000:.3f} |")
    for name, item in current["residuals"].items():
        lines.append(f"| {name}（残差） | — | {item['median'] * 1000:.3f} |")
    lines += ["", "SSG 局部释放残差是 SSG 总时间减去显式子阶段，含大上下文、文档、目录等局部对象释放及少量未分配的尾部工作；宿主残差同时含宿主准备、观察结果收集与 Vault 释放；发布余项同时含安全检查、目录交换和旧输出清理，不是独立 allocator profile。", "",
              "## 3. comemo：同进程完整站点组装", "",
              "同一个带 profiling feature 的二进制切换三种模式。每种模式三次进程，每次三轮新建 Vault；首轮计算缓存为空，后两轮保留进程级缓存。组装计时不含最后一次输出验证副本写盘。此 example 使用 FsResources 直接读取整份 Notist.toml，而普通 CLI 使用内存合成的 target 配置，两者的绝对组装时间不可互相相减；这里仅比较 example 内部的三种模式。", "",
              "| 模式 | 首轮 ms（中位数／范围） | 后两轮 ms（中位数／范围） | 首轮 compile_work / bind_work 次数 | 后续 compile_work / bind_work 次数 |",
              "| --- | ---: | ---: | ---: | ---: |"]
    for mode, data in summary["module_rounds"].items():
        a, b = data["first_ms"], data["later_ms"]
        counts = []
        for key in ["compile_work", "bind_work"]:
            counts.append(statistics.median(s["rounds"][0]["stages"][key]["calls"] for s in data["runs"]))
        for key in ["compile_work", "bind_work"]:
            counts.append(statistics.median(r["stages"][key]["calls"] for s in data["runs"] for r in s["rounds"][1:]))
        lines.append(f"| {mode} | {a['median']:.2f} ({a['min']:.2f}–{a['max']:.2f}) | {b['median']:.2f} ({b['min']:.2f}–{b['max']:.2f}) | {counts[0]:.0f} / {counts[1]:.0f} | {counts[2]:.0f} / {counts[3]:.0f} |")
    lines += ["", "XIV DB 生产模板站点没有 notc 导入，compile 实际执行为零。开启绑定缓存还包含空 BindingSpec 的共享空作用域 fast path；零 bind_work 不能表述为大量模块导入的缓存命中。三种模式的组装时间范围重叠，本轮未测得稳定的整站收益。普通 CLI 每次为新进程，不能用同进程热缓存解释其耗时差异。", "",
              "## 4. 产物与默认额度", "",
              "- 18 个点都通过消费者校验器：完整物品集合、513 页目录分页、全部本地 HTML 链接；详情不依赖 JavaScript。",
              "- 所有 Tera 及以后对照点的完整路径集合和逐文件 SHA-256 相同。探针版第一轮也逐文件与对应普通版本比较；同进程组装的最终完整输出副本也与普通版本逐文件一致。",
              "- Handlebars/Tera 的全部 HTML 解析事件相同（只归一化缩进空白、属性顺序和字符实体），非 HTML 文件字节相同。",
              f"- CSV SHA-256：`{manifest['csv_sha256']}`。",
              f"- Tera／当前完整产物聚合 SHA-256：`{current['validation']['sha256']}`。", "",
              "资源版本的生产默认 requests=16,384 无法完成该完整站点；成功比较仅在临时 CLI 显式设 1,000,000 请求，仍正常授权、版本检查与计费。默认额度失败单列如下，不参与加速比：", "",
              "| 版本 | 返回码 | 到失败的 wall s |", "| --- | ---: | ---: |"]
    for r in runs["default_budget"]:
        lines.append(f"| {r['case']} | {r['returncode']} | {r['wall_s']:.3f} |")
    method = runs["method"]
    lines += ["", "## 5. 方法、可复现性与限制", "",
              f"- {method['cpu_model']}；{method['system']}；rustc 1.97.1；Python {method['python']}。",
              f"- CPU {method['cpu']}，频率策略 `{json.dumps(method['cpu_frequency_policy'], ensure_ascii=False)}`；没有固定频率。",
              "- release debug=0、incremental=0、jobs=2。每个普通版本独立 Cargo target；编译和正式计时不并发。曾发现共享 target 的旧源码时间戳误复用，初轮全部弃用，重新隔离构建后才测量。",
              "- wall 含进程启动、Python prepare、Rust 管线、模板、文件发布及旧输出清理。温文件缓存、不做 fsync；不代表冷盘、增量构建或持久化写入速度。",
              "- wait4 的 ru_maxrss 单位 KiB，非进程树同时内存总和。user/system 也按 wait4 口径记录。",
              "- 前七个引擎恢复当时未提交的 publication 路径优化，避免拿未优化的平方级路径扫描当基线。存档 Handlebars 的上下文复用未提交源码未恢复，表中单独标明。",
              "- Python 三档脚本通过消费者 git 历史提取；品牌、CSV、静态搜索索引、内容及输出路径固定，未引入后续项目更名。每个点是累计版本，收益与顺序有关，不能当成相互独立的优化收益。",
              "- 当前工作区包括尚未提交的 str、arr.len、for 等语言能力，但这里仍运行同一生产模板负载。另一次约 53–56 s 的 notc 正文验收是不同宿主与页面模型，不作为本表回退基线。",
              f"- 结果目录：`{out}`。普通二进制、源码快照、探针补丁、固定输入、逐次 stdout/stderr、wait4 原始记录与逐文件指纹均保留。", "",
              "复现使用新目录（已有目录可续跑原条件下的 build/run/profiles/report；改变条件应另开目录）：", "",
              "```sh", "python3 tools/profile_xiv_history.py prepare --out /tmp/opencode/xiv-history-NEW",
              "python3 tools/profile_xiv_history.py build --out /tmp/opencode/xiv-history-NEW",
              "python3 tools/profile_xiv_history.py measure --out /tmp/opencode/xiv-history-NEW --runs 5 --cpu 8", "```", "",
              "机器可读汇总：`summary.json`、`summary.csv`；原始记录：`runs.json`、`profiles.json`。", ""]
    (out / "report.md").write_text("\n".join(lines))


def dispatch(args):
    if args.action == "measure":
        profile_build(args)
        run(args)
        profiles(args)
        report(args)
        return
    {"profile-build": profile_build, "run": run, "profiles": profiles, "report": report}[args.action](args)
