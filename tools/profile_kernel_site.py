#!/usr/bin/env python3
"""Profile the current kernel on a frozen full consumer, without editing either checkout.

Release timing, coarse attribution, detailed attribution and statistical CPU
sampling are separate runs. No workload reduction or optimization is applied.
"""
import argparse
import difflib
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import time

import profile_icexiv_refresh as refresh
from profile_current import parse_detail
from profile_xiv_history import git, once, save
from xiv_history_probes import enter_function

REPO = Path(__file__).resolve().parents[1]
KINDS = ["normal", "coarse", "detail"]
ENV = dict(CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2", CARGO_PROFILE_RELEASE_DEBUG="1")


def inventory(path):
    return refresh.inventory(path)


def instrument(source, detailed):
    refresh.instrument(source, detailed)
    prefix = "notist::xiv_profile"
    path = source / "crates/notist-cli/src/ssg.rs"
    text = path.read_text()
    text = once(text, "    result\n        .map(|artifact|", "    let outcome = result\n        .map(|artifact|")
    text = once(text, "        .map_err(|e| (e, observed))\n}",
                f'        .map_err(|e| (e, observed));\n    {prefix}::measure("vault.drop", || drop(vault));\n    outcome\n}}')
    text = once(text, "            ExitCode::SUCCESS", f'            {prefix}::measure("artifact.drop", || drop(result));\n            ExitCode::SUCCESS')
    path.write_text(text)
    path = source / "crates/notist-ssg/src/build.rs"
    text = path.read_text()
    # Explicitly time selected automatic destructors in their original relative
    # reverse-local order, after constructing the return value. No trees or
    # outputs are discarded early to improve the measured memory/latency.
    drops = "\n".join(f'    {prefix}::measure("ssg.drop.{name}", || drop({name}));'
                      for name in ["values", "dirs", "components", "by_source", "engine"])
    text = once(text, "    drop(phase);\n    result\n}", f"    drop(phase);\n{drops}\n    result\n}}")
    text = once(text, 'measure("pipeline.analyze", ||',
                'measure("pipeline.analyze", || notist::xiv_profile::measure(profile_page(&route.path, "analyze"), ||')
    text = once(text, 'vault.analyze_document_with_resource_root(&route.source, &text, &content))?;',
                'vault.analyze_document_with_resource_root(&route.source, &text, &content)))?;')
    text = once(text, 'measure("pipeline.evaluate", ||',
                'measure("pipeline.evaluate", || notist::xiv_profile::measure(profile_page(&route.path, "evaluate"), ||')
    text = once(text, 'vault.evaluate_document(&handle, &analysis))?;',
                'vault.evaluate_document(&handle, &analysis)))?;')
    text += '''
fn profile_page(path: &Path, stage: &str) -> &'static str {
    let item = path.parent() == Some(Path::new("items"))
        && path.file_stem().and_then(|s| s.to_str()).is_some_and(|s| s.bytes().all(|c| c.is_ascii_digit()));
    let catalog = path.starts_with("items") && !item;
    match (stage, item, catalog) {
        ("analyze", true, _) => "pages.item.analyze",
        ("analyze", _, true) => "pages.catalog.analyze",
        ("analyze", _, _) => "pages.other.analyze",
        ("evaluate", true, _) => "pages.item.evaluate",
        ("evaluate", _, true) => "pages.catalog.evaluate",
        ("evaluate", _, _) => "pages.other.evaluate",
        ("html", true, _) => "pages.item.html",
        ("html", _, true) => "pages.catalog.html",
        _ => "pages.other.html",
    }
}
'''
    if detailed:
        text = once(text, 'measure("body.html", ||',
                    'measure("body.html", || notist::xiv_profile::measure(profile_page(&document.page.route.path, "html"), ||')
        text = once(text, '.render_with_diagnostics(&document.root));',
                    '.render_with_diagnostics(&document.root)));')
        for name, label in [("new", "document.prepare")]:
            document = source / "crates/notist-ssg/src/document.rs"
            document.write_text(enter_function(document.read_text(), name,
                                 f'let _profile = notist::xiv_profile::span("{label}");'))
    path.write_text(text)


def prepare(args):
    out = args.out
    if out.exists() or out.is_relative_to(REPO) or out.is_relative_to(args.consumer):
        raise ValueError("use a fresh external output directory")
    for name in ["sources", "bin", "logs", "reproducer"]:
        (out / name).mkdir(parents=True, exist_ok=True)
    manifest = dict(revision=git(REPO, "rev-parse", "HEAD").decode().strip(),
                    status=git(REPO, "status", "--short").decode(),
                    consumer_revision=git(args.consumer, "rev-parse", "HEAD").decode().strip(),
                    consumer_status=git(args.consumer, "status", "--short").decode(),
                    rustc=subprocess.check_output(["rustc", "-Vv"], text=True),
                    system=platform.platform(), affinity=sorted(os.sched_getaffinity(0)),
                    build_env=ENV, started_at=time.time(), binaries={},
                    note="fresh CLI process per run, application caches start empty; filesystem caches uncontrolled; profiling runs are not clean timing")
    (out / "workspace.patch").write_bytes(git(REPO, "diff", "--binary", "HEAD"))
    refresh.freeze(out / "sources/normal")
    for name in refresh.INPUTS:
        src, dst = args.consumer / name, out / "site" / name
        if not src.exists():
            continue
        dst.parent.mkdir(parents=True, exist_ok=True)
        if src.is_dir():
            shutil.copytree(src, dst, symlinks=True, ignore=shutil.ignore_patterns("__pycache__"))
        else:
            shutil.copy2(src, dst)
    manifest["inputs"] = inventory(out / "site")
    manifest["input_symlinks"] = {str(p.relative_to(out / "site")): str(p.readlink())
                                  for p in (out / "site").rglob("*") if p.is_symlink()}
    export = json.loads((out / "site/inputs/game-export/manifest.json").read_text())
    handwritten = [p for p in (out / "site/content").rglob("*") if p.is_file()
                   and p.suffix in [".not", ".md"]
                   and not any(part.startswith(".") for part in p.relative_to(out / "site/content").parts)]
    manifest["workload"] = dict(items=export["itemCount"], catalog_pages=math.ceil(export["itemCount"] / 100),
                                handwritten_pages=len(handwritten), game_version=export["source"]["gameVersion"])
    manifest["expected_pages"] = export["itemCount"] + math.ceil(export["itemCount"] / 100) + len(handwritten)
    for kind in KINDS[1:]:
        source = out / "sources" / kind
        shutil.copytree(out / "sources/normal", source)
        instrument(source, kind == "detail")
        patch = []
        for path in sorted(source.rglob("*.rs")):
            relative = path.relative_to(source)
            original = out / "sources/normal" / relative
            before = original.read_text() if original.exists() else ""
            patch.extend(difflib.unified_diff(before.splitlines(True), path.read_text().splitlines(True),
                         fromfile="a/" + str(relative), tofile="b/" + str(relative)))
        (out / f"{kind}.patch").write_text("".join(patch))
    manifest["sources"] = {kind: inventory(out / "sources" / kind) for kind in KINDS}
    for name in ["profile_kernel_site.py", "profile_icexiv_refresh.py", "profile_current.py",
                 "profile_xiv_history.py", "xiv_history_probes.py", "current_profile_probe.rs"]:
        shutil.copy2(REPO / "tools" / name, out / "reproducer" / name)
    manifest["reproducer"] = inventory(out / "reproducer")
    save(out / "manifest.json", manifest)
    print("frozen", manifest["revision"], manifest["workload"], flush=True)


def guard(args):
    manifest = json.loads((args.out / "manifest.json").read_text())
    assert inventory(args.out / "reproducer") == manifest["reproducer"]
    for kind, expected in manifest["sources"].items():
        assert inventory(args.out / "sources" / kind) == expected, kind
    for kind, expected in manifest["binaries"].items():
        assert refresh.digest(args.out / "bin" / kind) == expected, kind
    actual = inventory(args.out / "site")
    links = {str(p.relative_to(args.out / "site")): str(p.readlink())
             for p in (args.out / "site").rglob("*") if p.is_symlink() and "dist" not in p.relative_to(args.out / "site").parts}
    assert links == manifest["input_symlinks"]
    assert {p: v for p, v in actual.items() if not p.startswith("dist/")} == manifest["inputs"]
    return manifest


def build(args):
    manifest = guard(args)
    target = args.out / "target"
    env = dict(os.environ, **ENV, CARGO_TARGET_DIR=str(target))
    for kind in KINDS:
        if kind in manifest["binaries"]:
            continue
        command = ["cargo", "build", "--release", "--locked", "--offline", "-p", "notist-cli", "--bin", "notist"]
        if kind != "normal":
            command += ["--features", "profiling"]
        with (args.out / "logs" / f"build-{kind}.log").open("wb") as log:
            metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version=1"],
                                  cwd=args.out / "sources" / kind, env=env))
            packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
            # The same workspace packages at copied paths can have older mtimes
            # than the previous build. Clean only our disposable workspace
            # products, retaining third-party dependency artifacts.
            subprocess.run(["cargo", "clean", "--release", *[v for p in packages for v in ["-p", p]]],
                           cwd=args.out / "sources" / kind, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            subprocess.run(command, cwd=args.out / "sources" / kind, env=env,
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(target / "release/notist", args.out / "bin" / kind)
        manifest["binaries"][kind] = refresh.digest(args.out / "bin" / kind)
        save(args.out / "manifest.json", manifest)
        print("built", kind, flush=True)


def process(args, kind, label, prefix=(), extra_env=None):
    manifest = json.loads((args.out / "manifest.json").read_text())
    assert refresh.digest(args.out / "bin" / kind) == manifest["binaries"][kind]
    env = dict(refresh.environment(), **(extra_env or {}))
    initial = not (args.out / "site/dist/icexiv-db").exists()
    command = [*prefix, str(args.out / "bin" / kind), "build"]
    started = time.perf_counter()
    with (args.out / "logs" / f"{label}.stdout").open("wb") as stdout, (args.out / "logs" / f"{label}.stderr").open("wb") as stderr:
        child = subprocess.Popen(command, cwd=args.out / "site", env=env, stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    row = dict(kind=kind, label=label, command=command, initial_publication=initial,
               wall_s=time.perf_counter()-started, user_s=usage.ru_utime, system_s=usage.ru_stime,
               maxrss_kib=usage.ru_maxrss, major_faults=usage.ru_majflt,
               involuntary_context_switches=usage.ru_nivcsw, exit=child.returncode,
               load=Path("/proc/loadavg").read_text().strip())
    save(args.out / "logs" / f"{label}.json", row)
    if child.returncode:
        raise RuntimeError(f"failed {label}: see logs/{label}.stderr")
    output = inventory(args.out / "site/dist/icexiv-db")
    assert sum(p.endswith(".html") for p in output) == manifest["expected_pages"]
    expected = args.out / "output-inventory.json"
    if expected.exists():
        assert output == json.loads(expected.read_text()), "complete output mismatch"
    else:
        save(expected, output)
    row["output_files"] = len(output)
    row["output_bytes"] = sum(v[0] for v in output.values())
    if kind != "normal":
        row["metrics"], row["edges"], row["modules"] = parse_detail((args.out / "logs" / f"{label}.stderr").read_text())
    save(args.out / "logs" / f"{label}.json", row)
    print(label, {k: row[k] for k in ["wall_s", "user_s", "system_s", "maxrss_kib", "output_files"]}, flush=True)
    return row


def measure(args):
    guard(args)
    destination = args.out / "samples.json"
    if destination.exists():
        raise ValueError("do not overwrite an existing cohort")
    rows = []
    for kind, label in [("normal", "initial-normal"), *[("normal", f"repeat-{i}-normal") for i in range(3)],
                        ("coarse", "probe-0-coarse"), ("detail", "probe-0-detail"),
                        ("detail", "probe-1-detail"), ("coarse", "probe-1-coarse")]:
        rows.append(process(args, kind, label))
        save(destination, rows)
    guard(args)


def sample(args):
    guard(args)
    if not args.perf or not args.perf.is_file():
        raise ValueError("--perf must name an installed perf executable")
    data = args.out / "cpu.data"
    process(args, "normal", "cpu-sample", [str(args.perf), "record", "-F", "199", "-e", "cpu-clock:u",
            "--call-graph", "dwarf,16384", "-o", str(data), "--"])
    for children in [False, True]:
        command = [str(args.perf), "report", "--stdio", "--percent-limit", "0.2", "-i", str(data),
                   "--sort", "comm,dso,symbol", "--children" if children else "--no-children"]
        with (args.out / ("cpu-children.txt" if children else "cpu-self.txt")).open("wb") as log:
            subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, check=True)
    guard(args)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "build", "measure", "sample"])
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--consumer", type=Path, default=Path("/home/azurice/Files/_ffxiv/icexiv-db"))
    parser.add_argument("--perf", type=Path)
    args = parser.parse_args()
    args.out = args.out.resolve()
    globals()[args.action](args)
