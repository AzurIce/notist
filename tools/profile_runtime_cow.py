#!/usr/bin/env python3
"""Validate the production lookup/COW changes against a frozen dirty worktree.

Only the five runtime files are overlaid onto the pre-change snapshot. CLI and
other concurrent edits must be identical in both sides of this comparison.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import random
import re
import shutil
import statistics
import subprocess
import threading
import time

FILES = [
    "crates/notist-core/src/shared.rs",
    "crates/notist-core/src/evaluation.rs",
    "crates/notist-pipeline/src/code/check.rs",
    "crates/notist-pipeline/src/code/eval.rs",
    "crates/notist-pipeline/src/code/instance_cache.rs",
]
ENV = {"CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "2", "CARGO_PROFILE_RELEASE_DEBUG": "1"}
KINDS = ["before", "after"]


def save(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def inventory(root):
    result = {}
    for path in sorted(root.rglob("*")):
        if path.is_file() and "__pycache__" not in path.parts:
            result[str(path.relative_to(root))] = [path.stat().st_size, digest(path)]
    return result


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def prepare(args):
    out = args.out
    manifest = json.loads((out / "manifest.json").read_text())
    before, after = (out / "sources" / kind for kind in KINDS)
    assert inventory(before) == manifest["before_sources"]
    assert not after.exists() and not (out / "site").exists()
    shutil.copytree(before, after)
    for relative in FILES:
        shutil.copy2(args.repo / relative, after / relative)
    manifest["after_sources"] = inventory(after)
    changed = {name for name in manifest["before_sources"].keys() | manifest["after_sources"].keys()
               if manifest["before_sources"].get(name) != manifest["after_sources"].get(name)}
    assert changed == set(FILES), changed
    manifest["runtime_files"] = FILES
    previous = json.loads((args.cohort / "manifest.json").read_text())
    shutil.copytree(args.cohort / "site", out / "site", ignore=shutil.ignore_patterns("dist", "__pycache__"))
    assert inventory(out / "site") == previous["inputs"]
    manifest["inputs"] = previous["inputs"]
    shutil.copy2(args.cohort / "output-inventory.json", out / "output-inventory.json")
    shutil.copy2(Path(__file__), out / "reproducer.py")
    manifest["reproducer_sha256"] = digest(out / "reproducer.py")
    manifest["rounds"] = args.rounds
    manifest["cpu_affinity"] = sorted(os.sched_getaffinity(0))
    manifest["rustc"] = subprocess.check_output(["rustc", "-Vv"], text=True)
    manifest["load_at_prepare"] = Path("/proc/loadavg").read_text()
    save(out / "manifest.json", manifest)
    print("prepared exactly five production file changes", flush=True)


def guard(args):
    out = args.out
    manifest = json.loads((out / "manifest.json").read_text())
    assert digest(out / "reproducer.py") == manifest["reproducer_sha256"]
    for kind in KINDS:
        assert inventory(out / "sources" / kind) == manifest[kind + "_sources"]
    actual = inventory(out / "site")
    assert {k: v for k, v in actual.items() if not k.startswith("dist/")} == manifest["inputs"]
    return manifest


def build(args):
    out = args.out
    manifest = guard(args)
    env = dict(os.environ, **ENV, CARGO_TARGET_DIR=str(out / "target"))
    for kind in KINDS:
        source = out / "sources" / kind
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=source, env=env))
        packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
        clean = ["cargo", "clean", "--release", *[v for p in packages for v in ["-p", p]]]
        if kind in manifest["binaries"]:
            assert digest(out / "bin" / kind) == manifest["binaries"][kind]
        else:
            with (out / "logs" / f"build-{kind}.log").open("wb") as log:
                # Archive mtimes in copied workspaces must not select stale local artifacts.
                subprocess.run(clean, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
                subprocess.run(["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist"],
                               cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            shutil.copy2(out / "target/release/notist", out / "bin" / kind)
            manifest["binaries"][kind] = digest(out / "bin" / kind)
            save(out / "manifest.json", manifest)
        # CLI integration fixtures resolve ../../modules from a temp project
        # under the checkout's target/, independently of CARGO_TARGET_DIR.
        (source / "target").mkdir(exist_ok=True)
        tested = manifest.setdefault("test_modes", {}).setdefault(kind, [])
        for features in ["default", "all"]:
            if features in tested:
                continue
            with (out / "logs" / f"tests-{kind}-{features}.log").open("wb") as log:
                # Hundreds of debug test executables need not coexist twice.
                subprocess.run(clean, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
                command = ["cargo", "test", "--locked", "--release", "--workspace", "--lib", "--tests"]
                if features == "all":
                    command += ["--all-features"]
                subprocess.run(command + ["--", "--test-threads=1"], cwd=source, env=env,
                               stdout=log, stderr=subprocess.STDOUT, check=True)
            tested.append(features)
            save(out / "manifest.json", manifest)
        print("built and tested", kind, flush=True)


def semantic(args):
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
    (fixture / "values.json").write_text("[1,2,3]\n")
    result = {}
    for name, source in cases.items():
        (fixture / "lib.notc").write_text(source)
        values = []
        for kind in KINDS:
            child = subprocess.run([str(out / "bin" / kind), "--eval", "json", "doc.not"],
                                   cwd=fixture, text=True, capture_output=True, check=True)
            actual = json.loads(child.stdout)
            assert "core" in actual
            save(out / "logs" / f"semantic-{name}-{kind}.json", actual)
            values.append(actual)
        assert values[0] == values[1], name
        assert values[0]["evaluation_diagnostics"] == (["inner failure"] if name == "failure" else [])
        result[name] = {"source": source, "identical": True}
    save(out / "semantic.json", result)
    print("four complete analysis/evaluation JSON fixtures matched", flush=True)


def process(args, kind, label, rss=False):
    out = args.out
    env = dict(os.environ, NO_COLOR="1", TERM="dumb", PYTHONDONTWRITEBYTECODE="1")
    for name in ["LD_PRELOAD", "RESOURCE_PROFILE", "RESOURCE_DIAGNOSTIC", "XIV_PROFILE_REQUESTS", "EVAL_AB_OBSERVER"]:
        assert not env.get(name), name
    points, stop = [], threading.Event()
    start = time.perf_counter()
    with (out / "logs" / f"{label}.stdout").open("wb") as stdout, (out / "logs" / f"{label}.stderr").open("wb") as stderr:
        child = subprocess.Popen([str(out / "bin" / kind), "build"], cwd=out / "site", env=env, stdout=stdout, stderr=stderr)

        def sample():
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
    row = {"kind": kind, "label": label, "wall_s": wall, "user_s": usage.ru_utime,
           "system_s": usage.ru_stime, "exit": child.returncode, "major_faults": usage.ru_majflt,
           "minor_faults": usage.ru_minflt, "involuntary_context_switches": usage.ru_nivcsw}
    assert child.returncode == 0, row
    assert inventory(out / "site/dist/icexiv-db") == json.loads((out / "output-inventory.json").read_text()), label
    row["output_verified"] = True
    if rss:
        row["peak_observed_rss_kib"] = max(point[1] for point in points)
        save(out / "logs" / f"{label}.rss.json", points)
    save(out / "logs" / f"{label}.json", row)
    print(label, round(wall, 4), flush=True)
    return row


def run(args):
    out = args.out
    manifest = guard(args)
    assert args.rounds == manifest["rounds"]
    assert not (out / "samples.json").exists()
    for kind in KINDS:
        assert digest(out / "bin" / kind) == manifest["binaries"][kind]
    # Populate the previously empty output directory before warmup/no-op timing.
    process(args, "before", "initial-publication")
    rows = []
    for rep in range(-1, args.rounds):
        for kind in KINDS if rep % 2 == 0 else KINDS[::-1]:
            row = process(args, kind, f"normal-{rep}-{kind}")
            row["round"] = rep
            rows.append(row)
            save(out / "samples.json", rows)
    diagnostics = []
    for rep in range(2):
        for kind in KINDS if rep == 0 else KINDS[::-1]:
            diagnostics.append(process(args, kind, f"rss-{rep}-{kind}", rss=True))
            save(out / "rss.json", diagnostics)
    subprocess.run(["python3", "scripts/check_site.py", "dist/icexiv-db"], cwd=out / "site", check=True)
    guard(args)


def report(args):
    out = args.out
    guard(args)
    rows = [r for r in json.loads((out / "samples.json").read_text()) if r["round"] >= 0]
    before = {r["round"]: r for r in rows if r["kind"] == "before"}
    paired = [100 * (before[r["round"]]["wall_s"] - r["wall_s"]) / before[r["round"]]["wall_s"]
              for r in rows if r["kind"] == "after"]
    rng = random.Random(20261104)
    bootstrap = sorted(statistics.median(rng.choices(paired, k=len(paired))) for _ in range(10000))
    result = {kind: {field: statistics.median(r[field] for r in rows if r["kind"] == kind)
                     for field in ["wall_s", "user_s", "system_s"]} for kind in KINDS}
    result.update(paired_gain_percent=paired, paired_median_gain_percent=statistics.median(paired),
                  paired_bootstrap_median_95_percent=[bootstrap[250], bootstrap[9749]],
                  faster_rounds=sum(p > 0 for p in paired))
    save(out / "summary.json", result)
    print(json.dumps(result, indent=2), flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "build", "semantic", "run", "report"])
    parser.add_argument("--out", required=True, type=Path)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument("--cohort", type=Path, default=Path("/tmp/opencode/notist-evaluator-ab-20261104"))
    parser.add_argument("--rounds", type=int, default=6)
    args = parser.parse_args()
    args.out = args.out.resolve()
    globals()[args.action](args)


if __name__ == "__main__":
    main()
