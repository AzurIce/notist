#!/usr/bin/env python3
"""Compare release binaries without changing consumer sources or dependency locks.

Build module_cache_profile with --features profiling. Run each implementation
with the same arguments and a different --label. CLI samples use fresh processes;
profile samples rebuild with fresh Vaults in one process. Peak RSS is wait4's
child high-water mark, NOT the sum of the process tree. Output hashing is untimed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def run(command, cwd, log, env, check=True):
    started = time.perf_counter()
    with log.with_suffix(".stdout").open("wb") as stdout, log.with_suffix(".stderr").open("wb") as stderr:
        child = subprocess.Popen(command, cwd=cwd, env=env, stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    result = {"command": list(map(str, command)), "cwd": str(cwd), "wall_s": time.perf_counter() - started,
              "user_s": usage.ru_utime, "system_s": usage.ru_stime, "peak_rss_kib": usage.ru_maxrss,
              "returncode": child.returncode}
    if child.returncode and check:
        raise RuntimeError(f"{result}\n{log.with_suffix('.stderr').read_text()[-5000:]}")
    return result


def manifest(root):
    digest = hashlib.sha256()
    count = size = html = 0
    for path in sorted(root.rglob("*")):
        if not path.is_file():
            continue
        contents = path.read_bytes()
        digest.update(path.relative_to(root).as_posix().encode() + b"\0")
        digest.update(hashlib.sha256(contents).digest())
        count += 1
        size += len(contents)
        html += path.suffix == ".html"
    return {"files": count, "html": html, "bytes": size, "sha256": digest.hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--label", required=True)
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--profile", required=True, type=Path)
    parser.add_argument("--xiv", type=Path)
    parser.add_argument("--docs", type=Path, help="frozen docs input (including its sibling modules/)")
    parser.add_argument("--cache-mode", choices=["baseline", "compile", "both"], default="both",
                        help="profiling-only mode; CLI samples still use the supplied binary")
    parser.add_argument("--out", type=Path, default=Path("/tmp/opencode/notist-module-cache-profile"))
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--rounds", type=int, default=3)
    args = parser.parse_args()
    if args.runs < 1 or args.rounds < 1:
        parser.error("--runs and --rounds must be positive")
    repo = Path(__file__).resolve().parents[1]
    docs = (args.docs or repo / "docs").resolve()
    dest = args.out / args.label
    if dest.exists():
        parser.error("result directory already exists; choose an unused --label to avoid stale artifacts")
    dest.mkdir(parents=True)
    env = dict(os.environ, NO_COLOR="1", TERM="dumb")
    results = {"label": args.label, "profile": {}, "cli": {}}
    cases = [("synthetic", "synthetic", "1000", repo), ("docs", "book", str(docs), repo)]
    if args.xiv:
        cases.append(("xiv", "site", str(args.xiv.resolve()), args.xiv.resolve()))
    for name, mode, root, cwd in cases:
        log = dest / f"profile-{name}"
        output = dest.resolve() / f"{name}-assembled-output"
        result = run([str(args.profile.resolve()), mode, root, str(args.rounds), str(output), args.cache_mode], cwd, log, env)
        result["rounds"] = [json.loads(line) for line in log.with_suffix(".stdout").read_text().splitlines()]
        if mode != "synthetic":
            result["manifest"] = manifest(output)
        results["profile"][name] = result
    for name, cwd in [("docs", docs)] + ([("xiv", args.xiv.resolve())] if args.xiv else []):
        samples = []
        output = dest.resolve() / f"{name}-output"
        for index in range(args.runs):
            samples.append(run([str(args.cli.resolve()), "build", "--out-dir", str(output)], cwd, dest / f"cli-{name}-{index}", env, check=False))
            if samples[-1]["returncode"]:
                break
        results["cli"][name] = {"samples": samples, "output": str(output), "manifest": manifest(output)}
    (dest / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps(results, ensure_ascii=False))


if __name__ == "__main__":
    main()
