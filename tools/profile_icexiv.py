#!/usr/bin/env python3
"""Profile the live IceXIV workload without changing either checkout.

All rewrites and builds are confined to a fresh external result directory.
Timings from normal, coarse and detailed binaries are never pooled.
"""
import argparse
import difflib
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import threading
import time

from profile_xiv_history import REPO, extract, git, once, save, sha
from xiv_history_probes import enter_function, measure_rhs, MODULE_STATS
from profile_current import parse_detail

KINDS = ["normal", "coarse", "detail"]
INPUTS = ["Notist.toml", "content", "data", "inputs", "modules", "scripts", "sources", "static", "templates"]


def inventory(root):
    result = {}
    for path in sorted(root.rglob("*")):
        if path.is_file() and "__pycache__" not in path.parts:
            digest = hashlib.sha256()
            with path.open("rb") as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    digest.update(block)
            result[str(path.relative_to(root))] = [path.stat().st_size, digest.hexdigest()]
    return result


def instrument(root, detail):
    core = root / "crates/notist-core/src"
    shutil.copy2(REPO / "tools/current_profile_probe.rs", core / "current_profile.rs")
    with (core / "lib.rs").open("a") as stream:
        stream.write("\n#[doc(hidden)]\npub mod current_profile;\n")
    with (root / "src/lib.rs").open("a") as stream:
        stream.write("\n#[doc(hidden)]\npub mod xiv_profile;\n")
    (root / "src/xiv_profile.rs").write_text(
        'pub use notist_core::current_profile::{span, measure, count};\npub struct Report;\n'
        'impl Drop for Report { fn drop(&mut self) { notist_core::current_profile::report();\n'
        + MODULE_STATS + '\n} }\n')

    def edit(relative, transform):
        path = root / relative
        path.write_text(transform(path.read_text()))

    def entries(relative, methods, prefix="notist_core::current_profile"):
        def transform(text):
            for method, label in methods:
                text = enter_function(text, method, f'let _probe = {prefix}::span("{label}");')
            return text
        edit(relative, transform)

    edit("crates/notist-cli/src/main.rs", lambda t: enter_function(t, "main",
        'let _report = notist::xiv_profile::Report;\nlet _root = notist::xiv_profile::span("cli.total");'))
    entries("crates/notist-cli/src/ssg.rs", [("assemble", "cli.assemble")], "notist::xiv_profile")
    entries("crates/notist-cli/src/publication.rs", [("publish", "cli.publish"), ("unchanged", "publish.compare")], "notist::xiv_profile")

    def site(t):
        t = enter_function(t, "build", 'let _total = notist::xiv_profile::span("ssg.total");\nlet phase = notist::xiv_profile::span("ssg.discover");')
        for anchor, label in [
            ('    let routes = crate::extensions::prepare', 'ssg.extension'),
            ('    let mut source_dirs: BTreeSet<_> =', 'ssg.source_directories'),
            ('    let mut documents = vec![];', 'ssg.pipeline'),
            ('    let mut assets = Assets::new(output);', 'ssg.assets'),
            ('    let by_source = DocumentIndex::new(&documents);', 'ssg.bodies'),
            ('    let dirs = directories(&pages, &source_dirs, &content);', 'ssg.shared_context'),
            ('    let workers = if pages.len()', 'ssg.templates'),
            ('    Ok(BuildOutput {', 'ssg.finish'),
        ]:
            t = once(t, anchor, f'    drop(phase);\n    let phase = notist::xiv_profile::span("{label}");\n' + anchor)
        t = measure_rhs(t, 'let (analysis, handle) =', ';\n        errors.extend(', 'pipeline.analyze')
        t = measure_rhs(t, 'let evaluation = ', ';\n        errors.extend(', 'pipeline.evaluate')
        # Keep the main-thread template span around worker joins; do not sum
        # worker CPU time into a wall-time partition or lose thread-local data.
        if detail:
            head, body = t.split('pub(crate) fn build', 1)
            for binding, end, label in [
                ('let (registry, environment) = ', ';\n        let rendered', 'body.registry'),
                ('let rendered = notist_html', ';\n        errors.extend(rendered', 'body.html'),
                ('let registrations = ', ';\n        if !registrations', 'body.components'),
            ]:
                if binding == 'let rendered = notist_html':
                    body = measure_rhs(body, 'let rendered = ', end, label)
                else:
                    body = measure_rhs(body, binding, end, label)
            t = head + 'pub(crate) fn build' + body
        t = once(t, '    Ok(BuildOutput {', '    let result = Ok(BuildOutput {')
        t = once(t, '        warnings,\n    })\n}', '        warnings,\n    });\n    drop(phase);\n'
            + '\n'.join(f'    notist::xiv_profile::measure("drop.{name}", || drop({name}));' for name in
                        ['values', 'dirs', 'components', 'by_source', 'engine', 'documents', 'handles'])
            + '\n    result\n}')
        return t
    edit("crates/notist-ssg/src/build.rs", site)
    edit("crates/notist-cli/src/ssg.rs", lambda t: once(once(t,
        '    result\n        .map(|artifact|', '    let result = result\n        .map(|artifact|'),
        '        .map_err(|e| (e, observed))\n}', '        .map_err(|e| (e, observed));\n    notist::xiv_profile::measure("drop.vault", || drop(vault));\n    result\n}'))
    entries("src/stdio.rs", [("exchange", "stdio.total")])
    edit("src/stdio.rs", lambda t: once(once(once(t,
        '    let input = serde_json::to_vec(request)', '    let phase = crate::xiv_profile::span("stdio.encode");\n    let input = serde_json::to_vec(request)'),
        '    let mut child = Command::new(program)', '    drop(phase);\n    let phase = crate::xiv_profile::span("stdio.process");\n    let mut child = Command::new(program)'),
        '    serde_json::from_slice(&output.stdout)', '    drop(phase);\n    let _decode = crate::xiv_profile::span("stdio.decode");\n    serde_json::from_slice(&output.stdout)'))
    if not detail:
        return
    for relative, methods in [
        ("src/environment/loader.rs", [("document", "environment.document"), ("visit", "environment.visit"), ("resolve_imports", "environment.imports"), ("new", "environment.graph")]),
        ("src/environment/config.rs", [("parse_config", "environment.config")]),
        ("src/resource_session.rs", [("source_bytes", "input.source"), ("load", "input.load"), ("data", "input.data"), ("register_source", "input.register")]),
        ("src/vault.rs", [("ensure_environment", "vault.environment"), ("analyze_document_with_resource_root", "vault.analyze"), ("evaluate_in", "vault.evaluate")]),
    ]:
        entries(relative, methods)
    for relative, methods in [
        ("frontend/markup/mod.rs", [("compile", "frontend.notist"), ("lower_content", "frontend.content")]),
        ("code/check.rs", [("document", "check.document"), ("install", "check.install")]),
        ("code/content.rs", [("content", "check.content")]),
        ("shape/mod.rs", [("shape", "check.shape"), ("shape_inline", "check.inline")]),
        ("materialize.rs", [("materialize", "check.materialize")]),
        ("definitions.rs", [("compile_uncached", "module.compile")]),
        ("modules.rs", [("bind_imports", "module.bind"), ("bind_cached", "module.bind_actual")]),
        ("code/data.rs", [("data_native", "data.native"), ("read_data", "data.read")]),
        ("code/eval.rs", [("run", "eval.run"), ("constant", "eval.comptime"), ("bind_content", "eval.bind_content"), ("present", "eval.present"), ("execute", "eval.execute"), ("implementation", "eval.implementation")]),
    ]:
        entries("crates/notist-pipeline/src/" + relative, methods)
    entries("crates/notist-core/src/data/parse.rs", [("parse", "data.parse")], "crate::current_profile")
    entries("crates/notist-core/src/data/mod.rs", [("decode", "data.decode")], "crate::current_profile")
    edit("crates/notist-core/src/data/cache.rs", lambda t: once(t,
        '        if let Some((_, result)) = state.parsed.get(&key) {',
        '        if let Some((_, result)) = state.parsed.get(&key) {\n            crate::current_profile::count("data.parse_hit", 1);'))


def prepare(args):
    out = args.out.resolve()
    if out.exists() or out.is_relative_to(REPO) or out.is_relative_to(args.consumer):
        raise ValueError("use a fresh result directory outside both checkouts")
    out.mkdir(parents=True)
    revision = git(REPO, "rev-parse", "HEAD").decode().strip()
    manifest = {"revision": revision, "consumer": str(args.consumer),
        "consumer_revision": git(args.consumer, "rev-parse", "HEAD").decode().strip(),
        "consumer_status": git(args.consumer, "status", "--porcelain=v1").decode(),
        "consumer_notist_pin": json.loads((args.consumer / "flake.lock").read_text())["nodes"]["notist"]["locked"],
        "cpu": platform.processor(), "system": platform.platform(),
        "affinity": sorted(os.sched_getaffinity(0)),
        "rustc": subprocess.check_output(["rustc", "-Vv"], text=True)}
    (out / "consumer.diff").write_bytes(git(args.consumer, "diff", "--binary", "HEAD"))
    for name in INPUTS:
        src, dst = args.consumer / name, out / "site" / name
        dst.parent.mkdir(parents=True, exist_ok=True)
        if src.is_dir():
            shutil.copytree(src, dst, ignore=shutil.ignore_patterns("__pycache__"))
        else:
            shutil.copy2(src, dst)
    manifest["inputs"] = inventory(out / "site")
    for kind in KINDS:
        source = out / "sources" / kind
        extract(REPO, revision, source)
        if kind != "normal":
            instrument(source, kind == "detail")
            patch = []
            for path in source.rglob("*.rs"):
                relative = path.relative_to(source)
                original = out / "sources/normal" / relative
                before = original.read_text() if original.exists() else ""
                patch.extend(difflib.unified_diff(before.splitlines(True), path.read_text().splitlines(True), fromfile="a/" + str(relative), tofile="b/" + str(relative)))
            (out / f"{kind}.patch").write_text("".join(patch))
    (out / "logs").mkdir()
    (out / "bin").mkdir()
    manifest["sources"] = {kind: inventory(out / "sources" / kind) for kind in KINDS}
    save(out / "manifest.json", manifest)


def build(args):
    for kind in KINDS:
        if (args.out / "bin" / kind).exists():
            continue
        env = dict(os.environ, CARGO_TARGET_DIR=str(args.out / "target"), CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2", CARGO_PROFILE_RELEASE_DEBUG="1")
        command = ["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist"]
        if kind != "normal":
            command += ["--features", "profiling"]
        with (args.out / "logs" / f"build-{kind}.log").open("wb") as log:
            # git archive preserves old mtimes. Shared-target Cargo freshness
            # must not accidentally reuse an uninstrumented local dependency.
            metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=args.out / "sources" / kind, env=env))
            packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
            subprocess.run(["cargo", "clean", "--release", *[flag for p in packages for flag in ["-p", p]]], cwd=args.out / "sources" / kind, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            subprocess.run(command, cwd=args.out / "sources" / kind, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(args.out / "target/release/notist", args.out / "bin" / kind)
        print("built", kind, flush=True)
    m = json.loads((args.out / "manifest.json").read_text())
    m["binaries"] = {kind: sha((args.out / "bin" / kind).read_bytes()) for kind in KINDS}
    save(args.out / "manifest.json", m)


def process(args, kind, label, prefix=(), env=None):
    started = time.perf_counter()
    with (args.out / "logs" / f"{label}.stdout").open("wb") as stdout, (args.out / "logs" / f"{label}.stderr").open("wb") as stderr:
        child = subprocess.Popen([*prefix, str(args.out / "bin" / kind), "build"], cwd=args.out / "site", env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1", NO_COLOR="1", TERM="dumb", **(env or {})), stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    row = {"kind": kind, "label": label, "wall_s": time.perf_counter() - started,
           "user_s": usage.ru_utime, "system_s": usage.ru_stime, "exit": child.returncode}
    assert child.returncode == 0, row
    if kind != "normal":
        row["metrics"], row["edges"], row["modules"] = parse_detail((args.out / "logs" / f"{label}.stderr").read_text())
    actual = inventory(args.out / "site/dist/icexiv-db")
    expected = args.out / "output-inventory.json"
    if expected.exists():
        assert actual == json.loads(expected.read_text()), "output differs"
    else:
        save(expected, actual)
    row["output_files"] = len(actual)
    row["output_bytes"] = sum(v[0] for v in actual.values())
    row["output_html"] = sum(p.endswith(".html") for p in actual)
    assert row["output_html"] == 51739, row
    row["command"] = [*prefix, str(args.out / "bin" / kind), "build"]
    save(args.out / "logs" / f"{label}.json", row)
    print(label, round(row["wall_s"], 3), flush=True)
    return row


def run(args):
    samples = []
    for rep in range(-1, 5):
        kinds = KINDS if rep < 3 else ["normal"]
        if rep >= 0 and rep % 2:
            kinds = kinds[::-1]
        for kind in kinds:
            row = process(args, kind, f"round-{rep}-{kind}")
            row["round"] = rep
            samples.append(row)
            save(args.out / "samples.json", samples)


def serve(args):
    # Observe the actual normal-binary serve timer, not an approximation from build.
    log = args.out / "logs/serve.stderr"
    with log.open("wb") as stderr:
        child = subprocess.Popen([str(args.out / "bin/normal"), "serve", "--port", "0", "--no-clear-screen"], cwd=args.out / "site", env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1", TERM="dumb", NO_COLOR="1"), stdout=subprocess.DEVNULL, stderr=stderr)
        rows = []
        trigger = args.out / "site/content/index.not"
        original = trigger.read_bytes()
        try:
            deadline = time.monotonic() + 600
            while len(rows) < 4:
                if child.poll() is not None:
                    raise RuntimeError(log.read_text())
                matches = re.findall(r"Updated · (\d+) pages · (\d+)ms", log.read_text())
                if len(matches) > len(rows):
                    pages, milliseconds = matches[len(rows)]
                    assert int(pages) == 51739
                    rows.append({"round": len(rows), "pages": int(pages), "elapsed_ms": int(milliseconds)})
                    save(args.out / "serve.json", rows)
                    print("serve", rows[-1], flush=True)
                    if len(rows) < 4:
                        time.sleep(2)
                        trigger.write_bytes(original)
                        deadline = time.monotonic() + 600
                if time.monotonic() > deadline:
                    raise TimeoutError(log.read_text())
                time.sleep(0.2)
        finally:
            child.terminate()
            child.wait(timeout=30)


def trace(args):
    process(args, "normal", "strace", ["strace", "-qq", "-f", "-c", "-e", "trace=%file,read,write,close,getdents64", "-o", str(args.out / "strace.txt")])


def memory(args):
    # Independent diagnostic: sample the Rust process, not the Python launcher
    # or the extension child. This is observed RSS, not allocation accounting.
    samples = []
    stopped = threading.Event()
    def observe():
        while not stopped.wait(0.02):
            try:
                text = Path(f"/proc/{child.pid}/status").read_text()
                rss = int(re.search(r"VmRSS:\s+(\d+)", text)[1])
                samples.append([time.perf_counter() - started, rss])
            except (FileNotFoundError, TypeError, ProcessLookupError):
                pass
    started = time.perf_counter()
    with (args.out / "logs/memory.stderr").open("wb") as stderr:
        child = subprocess.Popen([str(args.out / "bin/normal"), "build"], cwd=args.out / "site", stdout=subprocess.DEVNULL, stderr=stderr, env=dict(os.environ, PYTHONDONTWRITEBYTECODE="1"))
        observer = threading.Thread(target=observe)
        observer.start()
        try:
            assert child.wait() == 0
        finally:
            stopped.set()
            observer.join()
    assert inventory(args.out / "site/dist/icexiv-db") == json.loads((args.out / "output-inventory.json").read_text())
    save(args.out / "memory.json", {"interval_s": 0.02, "rust_only": True, "peak_observed_rss_kib": max(r[1] for r in samples), "samples": samples})


def python(args):
    wrapper = args.out / "python-wrapper"
    wrapper.mkdir()
    (wrapper / "python3").write_text(f'#!/bin/sh\nexec "{shutil.which("python3")}" -m cProfile -o "{args.out}/python.pstats" "$@"\n')
    (wrapper / "python3").chmod(0o755)
    process(args, "coarse", "python-cprofile", env={"PATH": str(wrapper) + os.pathsep + os.environ["PATH"]})


def ordered(args):
    """Control attribution without explicit SSG destructor ordering."""
    source = args.out / "sources/ordered"
    shutil.copytree(args.out / "sources/coarse", source)
    path = source / "crates/notist-ssg/src/build.rs"
    before = path.read_text()
    after = before
    for name in ['values', 'dirs', 'components', 'by_source', 'engine', 'documents', 'handles']:
        after = once(after, f'    notist::xiv_profile::measure("drop.{name}", || drop({name}));\n', '')
    path.write_text(after)
    (args.out / "ordered.patch").write_text(''.join(difflib.unified_diff(before.splitlines(True), after.splitlines(True))))
    env = dict(os.environ, CARGO_TARGET_DIR=str(args.out / "target"), CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2", CARGO_PROFILE_RELEASE_DEBUG="1")
    metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=source, env=env))
    packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
    with (args.out / "logs/build-ordered.log").open("wb") as log:
        subprocess.run(["cargo", "clean", "--release", *[flag for p in packages for flag in ["-p", p]]], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist", "--features", "profiling"], cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(args.out / "target/release/notist", args.out / "bin/ordered")
    shutil.copy2(Path(__file__), args.out / "ordered-reproducer.py")
    proof = {"sources": inventory(source), "binary": sha((args.out / "bin/ordered").read_bytes()),
             "reproducer_sha256": sha((args.out / "ordered-reproducer.py").read_bytes()), "samples": []}
    save(args.out / "ordered.json", proof)
    for rep in range(-1, 3):
        row = process(args, "ordered", f"ordered-{rep}")
        row["round"] = rep
        proof["samples"].append(row)
        save(args.out / "ordered.json", proof)
    assert inventory(source) == proof["sources"]


def report(args):
    samples = json.loads((args.out / "samples.json").read_text())
    result = {}
    for kind in KINDS:
        rows = [r for r in samples if r["kind"] == kind and r["round"] >= 0]
        result[kind] = {key: {"median": statistics.median(r[key] for r in rows), "min": min(r[key] for r in rows), "max": max(r[key] for r in rows)} for key in ["wall_s", "user_s", "system_s"]}
        if kind != "normal":
            result[kind]["metrics"] = {name: {field: statistics.mean(r["metrics"][name][field] for r in rows) for field in rows[0]["metrics"][name]} for name in rows[0]["metrics"]}
            result[kind]["modules"] = rows[0]["modules"]
    module_counts = [{k: v["calls"] for k, v in r["modules"].items()} for r in samples if r["kind"] != "normal"]
    assert all(counts == module_counts[0] for counts in module_counts), "module work changed"
    m = json.loads((args.out / "manifest.json").read_text())
    current = inventory(args.out / "site")
    assert {k: v for k, v in current.items() if not k.startswith("dist/")} == m["inputs"]
    assert all(sha((args.out / "bin" / k).read_bytes()) == v for k, v in m["binaries"].items())
    assert all(inventory(args.out / "sources" / k) == v for k, v in m["sources"].items())
    save(args.out / "summary.json", result)
    print(json.dumps({k: v["wall_s"] for k, v in result.items()}, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "build", "run", "serve", "trace", "python", "memory", "ordered", "report"])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--consumer", type=Path, default=Path("/home/azurice/Files/_ffxiv/icexiv-db"))
    args = parser.parse_args()
    args.out = args.out.resolve()
    globals()[args.action](args)


if __name__ == "__main__":
    main()
