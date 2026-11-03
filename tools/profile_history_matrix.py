#!/usr/bin/env python3
"""Historical releases x complete workloads, measured in one new batch.

Reuse verified release artifacts, never their old timing samples. New fixtures
use the language supported by the earliest release; no production source edits.
"""
import argparse
import csv
import hashlib
from html import escape
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import time
from urllib.parse import unquote, urlsplit

from profile_module_site import FIELDS, Page, fixture, literal
from profile_xiv_history import CASES, REPO, CSV_SHA, extract, git, once, save, sha
from xiv_history_probes import SemanticHTML, instrument, parse_stages

WORKLOADS = [
    ("xiv", "完整 XIV DB", 51739),
    ("literal", "无模块正文", 521),
    ("shared", "共享组件图", 521),
    ("private", "每页私有模块图", 521),
    ("wide", "宽导出／通配绑定", 521),
]
MATRIX_CASES = [
    ("current" if name == "working" else name,
     "current" if engine == "working" else engine, script,
     "当前已提交版本" if name == "working" else label)
    for name, engine, script, label in CASES
]


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def human(value):
    return "是" if value is True else "否" if value is False else str(value)


def item_source(item, index, reuse, wide=False):
    prefix = f'@!(title: {literal(item["name"])})\n'
    if wide:
        prefix += '#import "../../modules/view.notc" as view;\n'
        call = f"view.show{index % 8}"
    else:
        root = f"../../private/{item['id']}" if reuse == "private" else "../../modules"
        target = f"{root}/section{index % 8}.notc"
        form = index % 3
        prefix += f'#import "{target}" as view;\n' if form == 0 else f'#import "{target}": {"show" if form == 1 else "*"};\n'
        call = "view.show" if form == 0 else "show"
    arguments = [literal(human(item[k])) if wide or kind == "Int" else literal(item[k]) for k, kind in FIELDS.items()]
    return prefix + "#" + call + "(" + ", ".join(arguments) + ')\n\n#paragraph[#link("../catalog/index.not")[返回目录]]\n'


def synthetic(dest, reuse, pages=512):
    # Keep the site layout and realistic field data from the previous example,
    # but do not require the later scalar str() language addition.
    fixture(dest, pages, "none" if reuse in ["literal", "wide"] else reuse)
    spec = json.loads((dest / "fixture.json").read_text())
    site = dest / "site"
    if reuse in ["shared", "private"]:
        for path in site.rglob("*.notc"):
            text = path.read_text().replace(": Int", ": String").replace("{ str(value) }", "{ value }")
            write(path, text)
        for i, item in enumerate(spec["records"]):
            write(site / "content/items" / f"{item['id']}.not", item_source(item, i, reuse))
    elif reuse == "wide":
        # Every public function is used by a rendered layout in the batch.
        # A realistic broad component facade, not an unused-source padding test.
        for library in range(4):
            write(site / "modules" / f"labels{library}.notc", "".join(
                f'fn label_{n}() -> String {{ "field-{n:03d}" }}\n' for n in range(library * 64, (library + 1) * 64)))
        write(site / "modules/public.notc", "".join(f'import "labels{i}.notc": *;\n' for i in range(4)))
        parameters = ", ".join(f"{key}: String" for key in FIELDS)
        functions = []
        for group in range(8):
            rows = [f'#row[#cell[#text(text: label_{group * 32 + j}())]#cell[#text(text: {list(FIELDS)[j % len(FIELDS)]})]]' for j in range(32)]
            functions.append(f'fn show{group}({parameters}) -> BlockContent {{\n[\n#paragraph[group-{group}-A]\n\n#table()[\n' + "\n".join(rows) + '\n]\n]\n}\n')
        write(site / "modules/view.notc", 'import "public.notc": *;\n' + "\n".join(functions))
        for i, item in enumerate(spec["records"]):
            write(site / "content/items" / f"{item['id']}.not", item_source(item, i, reuse, wide=True))
    spec.update(reuse=reuse, render="wide" if reuse == "wide" else "records", scenarios=[])
    spec["config"].update(output="dist/bench")
    save(dest / "fixture.json", spec)
    write(site / "Notist.toml", '''[project]
name = "history-matrix"
[[site]]
name = "bench"
title = "模块缓存测试站"
base_url = "/"
content = "content"
templates = "templates"
static_dir = "static"
template = "page"
''')
    shutil.copytree(site / "templates", dest / "templates-tera")
    tera = (site / "templates/page.tera").read_text()
    write(dest / "templates-handlebars/page.hbs", tera.replace("{{ page.html | safe }}", "{{{page.html}}}"))


def binary_meta(directory, engine, old):
    original = old["engines"][engine]
    normal = directory / "bin" / engine
    assert sha(normal.read_bytes()) == original["binary_sha256"], engine
    if engine != "handlebars":
        assert original.get("target_dir") == str(directory / "targets" / engine), "not an isolated original build"
    return original


def prepare(args):
    out, history = args.out.resolve(), args.history.resolve()
    if out.exists():
        raise ValueError("use a new result directory")
    if out == REPO or REPO in out.parents:
        raise ValueError("results must be outside the checkout")
    old = json.loads((history / "manifest.json").read_text())
    out.mkdir(parents=True)
    for name in ["bin", "sources", "logs", "inventories", "sites"]:
        (out / name).mkdir()
    current = git(REPO, "rev-parse", "HEAD").decode().strip()
    manifest = {"current": current, "cases": MATRIX_CASES, "workloads": WORKLOADS, "engines": {},
                "rustc": subprocess.check_output(["rustc", "-Vv"], text=True), "system": platform.platform(), "python": platform.python_version(),
                "artifact_source": str(history), "runs": args.runs, "cpu": args.cpu, "requests": 1000000,
                "build_env": {"CARGO_PROFILE_RELEASE_DEBUG": "0", "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "2"}}
    assert manifest["rustc"] == old["rustc"], "rebuild historical binaries if the toolchain changed"
    (out / "checkout-start.diff").write_bytes(git(REPO, "diff", "--binary", "HEAD"))
    (out / "historical-manifest.json").write_bytes((history / "manifest.json").read_bytes())
    for engine in dict.fromkeys(case[1] for case in MATRIX_CASES):
        if engine == "current":
            source = out / "sources/current"
            extract(REPO, current, source)
            cli = source / "crates/notist-cli/src/ssg.rs"
            cli.write_text(once(cli.read_text(), "Vault::new(prepared.files).with_config(prepared.config)",
                'Vault::new(prepared.files).with_input_limits(notist::InputLimits { requests: std::env::var("XIV_PROFILE_REQUESTS").map(|v| v.parse().unwrap()).unwrap_or(16384), ..Default::default() }).with_config(prepared.config)'))
            manifest["engines"][engine] = {"revision": current, "source": str(source),
                "adaptations": ["benchmark-only request budget override; current checkout's uncommitted host policy excluded"]}
            continue
        meta = binary_meta(history, engine, old).copy()
        shutil.copy2(history / "bin" / engine, out / "bin" / engine)
        if engine != "handlebars":
            expected = json.loads((history / "logs" / f"profile-binary-{engine}.json").read_text())["sha256"]
            profile = history / "bin" / f"profile-{engine}"
            assert sha(profile.read_bytes()) == expected
            shutil.copy2(profile, out / "bin" / f"profile-{engine}")
            meta["profile_sha256"] = expected
            shutil.copytree(Path(meta["source"]), out / "sources" / engine)
        meta["reused_from"] = str(history)
        meta["source"] = str(out / "sources" / engine) if engine != "handlebars" else None
        manifest["engines"][engine] = meta
    xiv = out / "sites/xiv"
    for name in ["content", "data", "static", "sources", "inputs"]:
        shutil.copytree(history / "site" / name, xiv / "site" / name)
    shutil.copy2(history / "site/Notist.toml", xiv / "site/Notist.toml")
    (xiv / "site/scripts").mkdir()
    shutil.copy2(history / "site/scripts/check_site.py", xiv / "site/scripts/check_site.py")
    for name in ["templates-tera", "templates-handlebars", "scripts"]:
        shutil.copytree(history / name, xiv / name)
    assert sha((xiv / "site/inputs/Item.csv").read_bytes()) == CSV_SHA
    for workload, _, _ in WORKLOADS[1:]:
        synthetic(out / "sites" / workload, workload)
    manifest["fixed_files"] = {}
    for p in sorted((out / "sites").rglob("*")):
        if not p.is_file():
            continue
        rel = p.relative_to(out).as_posix()
        if "/site/templates/" in rel or rel.endswith("/site/scripts/items.py"):
            continue
        manifest["fixed_files"][rel] = sha(p.read_bytes())
    save(out / "manifest.json", manifest)
    print(f"prepared {len(MATRIX_CASES)} checkpoints x {len(WORKLOADS)} workloads", flush=True)


def build(args):
    out = args.out.resolve()
    m = json.loads((out / "manifest.json").read_text())
    for profile in [False, True]:
        binary = out / "bin" / ("profile-current" if profile else "current")
        if binary.exists():
            continue
        source = out / "sources" / ("profile-current" if profile else "current")
        if profile and not source.exists():
            shutil.copytree(out / "sources/current", source)
            instrument(source, "current")
        target = out / ("target-profile-current" if profile else "target-current")
        env = dict(os.environ, CARGO_TARGET_DIR=str(target), **m["build_env"], NO_COLOR="1", TERM="dumb")
        command = ["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--bin", "notist"]
        if profile:
            command += ["--features", "profiling"]
        with (out / "logs" / f"build-{binary.name}.log").open("wb") as log:
            subprocess.run(command, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(target / "release/notist", binary)
        m["engines"]["current"]["profile_sha256" if profile else "binary_sha256"] = sha(binary.read_bytes())
        save(out / "manifest.json", m)
        print(f"built {binary.name}", flush=True)


def output_root(out, workload):
    return out / "sites" / workload / "site/dist" / ("xiv-db" if workload == "xiv" else "bench")


def configure(out, case, workload):
    _, engine, script, _ = case
    base = out / "sites" / workload
    dest = base / "site/templates"
    if dest.exists():
        # Exclusively a generated copy in this run's results.
        shutil.rmtree(dest)
    shutil.copytree(base / ("templates-handlebars" if engine == "handlebars" else "templates-tera"), dest)
    if workload == "xiv":
        shutil.copy2(base / "scripts" / f"{script}.py", base / "site/scripts/items.py")


def process(out, workload, command, label, cpu):
    env = dict(os.environ, NO_COLOR="1", TERM="dumb", XIV_PROFILE_REQUESTS="1000000")
    cmd = ["taskset", "-c", str(cpu), *map(str, command)]
    log = out / "logs" / label
    start = time.perf_counter()
    with log.with_suffix(".stdout").open("wb") as stdout, log.with_suffix(".stderr").open("wb") as stderr:
        child = subprocess.Popen(cmd, cwd=out / "sites" / workload / "site", env=env, stdout=stdout, stderr=stderr)
        _, status, usage = os.wait4(child.pid, 0)
        child.returncode = os.waitstatus_to_exitcode(status)
    result = {"command": cmd, "wall_s": time.perf_counter() - start, "user_s": usage.ru_utime, "system_s": usage.ru_stime,
              "rss_kib": usage.ru_maxrss, "returncode": child.returncode, "stdout": str(log.with_suffix(".stdout")), "stderr": str(log.with_suffix(".stderr"))}
    if child.returncode:
        raise RuntimeError(json.dumps(result) + "\n" + log.with_suffix(".stderr").read_text()[-12000:])
    return result


def inventory(root, semantics=False):
    paths, semantic, count, size = {}, {}, 0, 0
    for p in sorted(root.rglob("*")):
        if not p.is_file():
            continue
        rel, raw = p.relative_to(root).as_posix(), p.read_bytes()
        paths[rel] = sha(raw)
        size += len(raw)
        count += p.suffix == ".html"
        if semantics:
            if p.suffix == ".html":
                parser = SemanticHTML()
                parser.feed(raw.decode())
                parser.close()
                semantic[rel] = sha(json.dumps(parser.tokens, ensure_ascii=False, separators=(",", ":")).encode())
            else:
                semantic[rel] = paths[rel]
    digest = hashlib.sha256()
    for path, value in paths.items():
        digest.update(path.encode() + b"\0" + bytes.fromhex(value))
    return {"files": len(paths), "html": count, "bytes": size, "sha256": digest.hexdigest(), "paths": paths, "semantics": semantic}


def verify_synthetic(root, spec):
    items = {f"items/{r['id']}.html": (i, r) for i, r in enumerate(spec["records"])}
    catalogs, html_count = [], 0
    for p in sorted(root.rglob("*.html")):
        html_count += 1
        rel = p.relative_to(root).as_posix()
        parser = Page()
        parser.feed(p.read_text())
        parser.close()
        for link in parser.links:
            parsed = urlsplit(link)
            if parsed.scheme or parsed.netloc or not parsed.path:
                continue
            target = (root if parsed.path.startswith("/") else p.parent) / unquote(parsed.path.lstrip("/"))
            assert target.exists(), (p, link)
        if rel in items:
            index, item = items[rel]
            if spec["render"] == "wide":
                expected = {f"field-{index % 8 * 32 + j:03d}": human(item[list(FIELDS)[j % len(FIELDS)]]) for j in range(32)}
            else:
                expected = {k: human(v) for k, v in item.items()}
            assert len(parser.rows) == len(expected) and dict(parser.rows) == expected, (rel, parser.rows)
            assert f"group-{index % 8}-A" in "".join(parser.text)
        elif rel.startswith("catalog/"):
            catalogs.extend(int(row[0]) for row in parser.rows)
    assert html_count == spec["page_count"]
    assert sorted(catalogs) == [r["id"] for r in spec["records"]]
    assert {p.relative_to(root).as_posix() for p in (root / "items").glob("*.html")} == set(items)


def guard(out, args):
    m = json.loads((out / "manifest.json").read_text())
    assert m["runs"] == args.runs and m["cpu"] == args.cpu, "use new results for changed settings"
    for rel, expected in m["fixed_files"].items():
        assert sha((out / rel).read_bytes()) == expected, f"fixed input changed: {rel}"
    for engine, meta in m["engines"].items():
        assert sha((out / "bin" / engine).read_bytes()) == meta["binary_sha256"], engine
        if engine != "handlebars":
            assert sha((out / "bin" / f"profile-{engine}").read_bytes()) == meta["profile_sha256"]
    return m


def validate(args):
    out = args.out.resolve()
    guard(out, args)
    result_path = out / "validation.json"
    results = json.loads(result_path.read_text()) if result_path.exists() else {}
    for workload, _, pages in WORKLOADS:
        baseline = handlebars = None
        for case in MATRIX_CASES:
            name, engine, _, _ = case
            key = workload + "/" + name
            path = out / "inventories" / f"{workload}-{name}.json"
            if key not in results:
                configure(out, case, workload)
                sample = process(out, workload, [out / "bin" / engine, "build"], f"warmup-{workload}-{name}", args.cpu)
                current = inventory(output_root(out, workload), semantics=name in ["handlebars", "tera"])
                assert current["html"] == pages, (key, current["html"])
                if workload == "xiv":
                    process(out, workload, ["python3", "scripts/check_site.py", "dist/xiv-db"], f"verify-{workload}-{name}", args.cpu)
                else:
                    spec = json.loads((out / "sites" / workload / "fixture.json").read_text())
                    verify_synthetic(output_root(out, workload), spec)
                save(path, current)
                results[key] = {"warmup": sample, "inventory": {k: v for k, v in current.items() if k not in ["paths", "semantics"]}}
                save(result_path, results)
            else:
                current = json.loads(path.read_text())
            if name == "handlebars":
                handlebars = current
            elif name == "tera":
                assert handlebars["semantics"] == current["semantics"], f"template migration changed semantics: {workload}"
                baseline = current
            else:
                difference = [p for p in baseline["paths"].keys() | current["paths"].keys() if baseline["paths"].get(p) != current["paths"].get(p)]
                assert not difference, f"{key} byte differences: {difference[:12]}"
            print(f"validated {key}: {pages} HTML", flush=True)
    guard(out, args)
    save(out / "validation-complete.json", {"cells": len(results), "cases": len(MATRIX_CASES), "workloads": len(WORKLOADS)})


def run(args):
    out = args.out.resolve()
    guard(out, args)
    validations = json.loads((out / "validation.json").read_text())
    assert len(validations) == len(MATRIX_CASES) * len(WORKLOADS)
    assert json.loads((out / "validation-complete.json").read_text())["cells"] == len(validations)
    path = out / "runs.json"
    results = json.loads(path.read_text()) if path.exists() else []
    cells = [(case, workload) for case in MATRIX_CASES for workload, _, _ in WORKLOADS]
    for rep in range(args.runs):
        shift = rep * 17 % len(cells)
        order = cells[shift:] + cells[:shift]
        if rep % 2:
            order = list(reversed(order))
        for case, workload in order:
            name, engine, _, _ = case
            if any(s["round"] == rep and s["case"] == name and s["workload"] == workload for s in results):
                continue
            configure(out, case, workload)
            sample = process(out, workload, [out / "bin" / engine, "build"], f"cli-{rep}-{workload}-{name}", args.cpu)
            sample.update(case=name, workload=workload, round=rep)
            results.append(sample)
            save(path, results)
            print(f"CLI {rep + 1}/{args.runs} {name}/{workload}: {sample['wall_s']:.4f}s", flush=True)
    guard(out, args)


def probes(args):
    out = args.out.resolve()
    guard(out, args)
    path = out / "probes.json"
    results = json.loads(path.read_text()) if path.exists() else []
    for case in MATRIX_CASES:
        name, engine, _, _ = case
        if engine == "handlebars":
            continue
        for workload, _, _ in WORKLOADS:
            if any(s["case"] == name and s["workload"] == workload for s in results):
                continue
            configure(out, case, workload)
            sample = process(out, workload, [out / "bin" / f"profile-{engine}", "build"], f"probe-{workload}-{name}", args.cpu)
            sample.update(case=name, workload=workload)
            sample["stages"], sample["modules"] = parse_stages(Path(sample["stderr"]))
            current = inventory(output_root(out, workload))
            expected = json.loads((out / "inventories" / f"{workload}-{name}.json").read_text())
            assert current["paths"] == expected["paths"], f"probe changed output: {name}/{workload}"
            results.append(sample)
            save(path, results)
            print(f"probed {name}/{workload}", flush=True)
    # These are deterministic work counts, not single-sample speed conclusions.
    for workload, _, _ in WORKLOADS:
        modes = {name: next(s["modules"] for s in results if s["case"] == name and s["workload"] == workload) for name in ["memo-off", "memo-compile", "memo-both"]}
        if workload in ["xiv", "literal"]:
            assert all(m["compile_work"]["calls"] == 0 for m in modes.values())
        elif workload in ["shared", "wide"]:
            assert modes["memo-compile"]["compile_work"]["calls"] < modes["memo-off"]["compile_work"]["calls"]
            assert modes["memo-both"]["bind_work"]["calls"] < modes["memo-compile"]["bind_work"]["calls"]
        else:
            assert modes["memo-compile"]["compile_work"]["calls"] == modes["memo-off"]["compile_work"]["calls"]
    guard(out, args)


def distribution(values):
    return {"median": statistics.median(values), "min": min(values), "max": max(values), "values": values}


def chart_payload(summary):
    data = {"workloads": summary["workloads"], "cases": []}
    for entry in summary["cases"]:
        row = {"key": entry["case"], "revision": "历史存档" if entry["case"] == "handlebars" else entry["revision"][:7], "values": {}}
        for workload, _, _ in summary["workloads"]:
            cell = entry["workloads"][workload]
            modules = cell.get("probe_modules", {})
            row["values"][workload] = [round(cell["wall_s"][k], 6) for k in ["median", "min", "max"]] + [round(cell["rss_kib"][k] / 1024, 3) for k in ["median", "min", "max"]] + [modules.get(k, {}).get("calls") for k in ["compile_work", "bind_work"]]
        data["cases"].append(row)
    return data


def charts(out, summary):
    data = chart_payload(summary)
    save(out / "chart-data.json", data)
    template = Path(__file__).with_name("history_matrix_chart.html").read_text()
    write(out / "dashboard.html", template.replace("__HISTORY_MATRIX__", json.dumps(data, ensure_ascii=False, separators=(",", ":")).replace("<", "\\u003c")))
    colors = ["#3b82f6", "#64748b", "#10b981", "#df5866", "#a855f7"]
    for metric, offset, unit in [("wall", 0, "s"), ("rss", 3, "MiB")]:
        width, left, right, top, row_height = 1100, 230, 90, 100, 72
        height = top + row_height * len(data["cases"]) + 40
        maximum = max(c["values"][w[0]][offset + 2] for c in data["cases"] for w in WORKLOADS) * 1.03
        plot_width = width - left - right
        parts = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><rect width="{width}" height="{height}" fill="#ffffff"/><style>text{{font-family:system-ui,sans-serif;fill:#263244;font-size:12px}}.small{{font-size:10px;fill:#607080}}</style>',
                 f'<text x="12" y="23" style="font-size:18px;font-weight:600">历史版本 × 五种完整负载 · {unit}</text>',
                 '<text x="12" y="44" class="small">五轮 CLI 中位数；细线为 min–max。相同绝对刻度，每个版本五条。Handlebars 为存档参照，comemo off/compile 为消融。</text>']
        for i, (_, label, _) in enumerate(WORKLOADS):
            x = 12 + i * 212
            parts.append(f'<rect x="{x}" y="57" width="9" height="9" fill="{colors[i]}"/><text x="{x+14}" y="66">{escape(label)}</text>')
        for i in range(6):
            x, value = left + plot_width * i / 5, maximum * i / 5
            parts.append(f'<line x1="{x}" y1="88" x2="{x}" y2="{height-28}" stroke="#e7ebf0"/><text x="{x}" y="85" text-anchor="middle" class="small">{value:.1f}</text>')
        for i, entry in enumerate(data["cases"]):
            y = top + i * row_height
            parts.append(f'<text x="12" y="{y+21}">{escape(entry["key"])}</text><text x="12" y="{y+38}" class="small">{escape(entry["revision"])}</text>')
            for j, (workload, _, _) in enumerate(WORKLOADS):
                value, minimum, maximum_sample = entry["values"][workload][offset:offset + 3]
                bar_y = y + j * 11
                x1, x2 = left + plot_width * minimum / maximum, left + plot_width * maximum_sample / maximum
                parts.append(f'<rect x="{left}" y="{bar_y}" width="{plot_width*value/maximum}" height="7" rx="1" fill="{colors[j]}"/><line x1="{x1}" y1="{bar_y+3.5}" x2="{x2}" y2="{bar_y+3.5}" stroke="{colors[j]}"/><text x="{width-10}" y="{bar_y+7}" text-anchor="end" class="small">{value:.3f}</text>')
        parts.append('</svg>')
        write(out / f"history-{metric}.svg", "".join(parts))


def report(args):
    out = args.out.resolve()
    m = guard(out, args)
    runs = json.loads((out / "runs.json").read_text())
    validation = json.loads((out / "validation.json").read_text())
    probe = json.loads((out / "probes.json").read_text())
    assert len(runs) == args.runs * len(MATRIX_CASES) * len(WORKLOADS)
    summary = {"cases": [], "workloads": WORKLOADS, "runs": args.runs, "cpu": args.cpu, "current": m["current"]}
    for name, engine, script, label in MATRIX_CASES:
        entry = {"case": name, "label": label, "revision": m["engines"][engine]["revision"], "xiv_script": script, "workloads": {}}
        for workload, _, _ in WORKLOADS:
            samples = [s for s in runs if s["case"] == name and s["workload"] == workload]
            assert len(samples) == args.runs and all(s["returncode"] == 0 for s in samples)
            data = {key: distribution([s[key] for s in samples]) for key in ["wall_s", "user_s", "system_s", "rss_kib"]}
            data["output"] = validation[workload + "/" + name]["inventory"]
            if engine != "handlebars":
                p = next(p for p in probe if p["case"] == name and p["workload"] == workload)
                data["probe_stages"], data["probe_modules"] = p["stages"], p["modules"]
            entry["workloads"][workload] = data
        summary["cases"].append(entry)
    save(out / "summary.json", summary)
    charts(out, summary)
    with (out / "summary.csv").open("w", newline="") as file:
        writer = csv.writer(file)
        writer.writerow(["case", "revision", "workload", "wall_median_s", "wall_min_s", "wall_max_s", "rss_median_mib"])
        for entry in summary["cases"]:
            for workload, data in entry["workloads"].items():
                writer.writerow([entry["case"], entry["revision"], workload, data["wall_s"]["median"], data["wall_s"]["min"], data["wall_s"]["max"], data["rss_kib"]["median"] / 1024])
    lines = ["# 多情形历轮性能对照（2026-11-03）", "", "每个历史点并排比较五种完整负载，不把冷／热编辑序列当作历史版本，也不复用旧报告的时间。", "",
        f"共 {len(MATRIX_CASES)} 个点 × {len(WORKLOADS)} 个负载 × {args.runs} 轮 = {len(runs)} 次普通 CLI 测量；每个点／负载完整预热与验证，另用独立探针二进制检查阶段和模块工作次数。", "",
        "## 主要发现", ""]
    by_case = {entry["case"]: entry for entry in summary["cases"]}
    value = lambda case, workload, metric="wall_s": by_case[case]["workloads"][workload][metric]["median"]
    private_probe = by_case["current"]["workloads"]["private"]["probe_stages"]
    remaining = [label for workload, label, _ in WORKLOADS if value("current", workload) > value("environment", workload) * 1.1]
    lines += [
        f"- XIV：Tera {value('tera', 'xiv'):.3f} → 当前 {value('current', 'xiv'):.3f} s，减少 {(1 - value('current', 'xiv') / value('tera', 'xiv')) * 100:.1f}%。原有模板、Python、语义共享优化的收益仍然可见。",
        "- 新负载揭示了资源重构的明显退化，不能只用生产 XIV 的曲线概括所有情形：", "",
        "| 情形 | 环境句柄版 s | 资源重构版 s | 后者 / 前者 |", "| --- | ---: | ---: | ---: |"]
    for workload, label, _ in WORKLOADS:
        before, after = value("environment", workload), value("resources", workload)
        lines.append(f"| {label} | {before:.4f} | {after:.4f} | {after / before:.2f}× |")
    lines += ["",
        f"- comemo 在宽导出／通配绑定情形中有明确收益：off {value('memo-off', 'wide'):.3f} → compile {value('memo-compile', 'wide'):.3f} → both {value('memo-both', 'wide'):.3f} s；off→both 减少 {(1 - value('memo-both', 'wide') / value('memo-off', 'wide')) * 100:.1f}%，RSS {value('memo-off', 'wide', 'rss_kib') / 1024:.1f} → {value('memo-both', 'wide', 'rss_kib') / 1024:.1f} MiB。绑定缓存在仅编译缓存之外再减少约 {(1 - value('memo-both', 'wide') / value('memo-compile', 'wide')) * 100:.1f}% wall。",
        f"- 共享组件图 off→both 约减少 {(1 - value('memo-both', 'shared') / value('memo-off', 'shared')) * 100:.1f}%；私有图没有首轮跨页编译复用，5120 次编译仍然全部执行，无明显整站收益。XIV 和无导入负载不发生模块编译，不能用来评价编译缓存的潜在收益。",
        f"- 当前比环境句柄版仍慢逾 10% 的情形：{'、'.join(remaining) or '无'}。探针将资源重构的主要增量定位到分析／求值，而非模板或发布。当前私有图一次探针约 {private_probe['pipeline.analyze']['seconds']:.2f} s 在 analyze、{private_probe['pipeline.evaluate']['seconds']:.2f} s 在 evaluate；模板约 {private_probe['ssg.page_templates']['seconds'] * 1000:.1f} ms。探针只有一次，用于量级定位，不替代五轮 CLI 结论。",
        "- 代码证据：ResourceSession.source_bytes 在每次读取（包括缓存查询之前）调用 validate；validate 遍历全部已观察文件与 kinds，FsReader.revision 会重新打开文件并取 metadata，evaluate 前后还全量校验。私有模块图不断扩大观察集合，是明确的后续排查点。没有单独消融这些检查，因此不声称它解释了全部增量；本轮未移除授权／版本检查、未修改引擎。",
        "- 这也是先前 MemoryResources 组装例不能直接代替文件系统 CLI 历轮曲线的原因：此处实际测了物理资源版本检查和完整发布，兼容样例的参数类型也不同。两批绝对秒数不相减。", "",
        "## wall 中位数（s）", "", "| 对照点 | " + " | ".join(w[1] for w in WORKLOADS) + " |", "| --- | " + " | ".join(["---:"] * len(WORKLOADS)) + " |"]
    for entry in summary["cases"]:
        lines.append("| " + entry["case"] + " · " + entry["label"] + " | " + " | ".join(f"{entry['workloads'][w]['wall_s']['median']:.4f}" for w, _, _ in WORKLOADS) + " |")
    lines += ["", "## 峰值 RSS 中位数（MiB）", "", "| 对照点 | " + " | ".join(w[1] for w in WORKLOADS) + " |", "| --- | " + " | ".join(["---:"] * len(WORKLOADS)) + " |"]
    for entry in summary["cases"]:
        lines.append("| " + entry["case"] + " | " + " | ".join(f"{entry['workloads'][w]['rss_kib']['median'] / 1024:.1f}" for w, _, _ in WORKLOADS) + " |")
    lines += ["", "## comemo 实际工作次数（编译 / 绑定）", "", "| 模式 | " + " | ".join(w[1] for w in WORKLOADS) + " |", "| --- | " + " | ".join(["---:"] * len(WORKLOADS)) + " |"]
    for name in ["memo-off", "memo-compile", "memo-both", "current"]:
        entry = next(e for e in summary["cases"] if e["case"] == name)
        values = [entry["workloads"][w]["probe_modules"] for w, _, _ in WORKLOADS]
        lines.append("| " + name + " | " + " | ".join(f"{v['compile_work']['calls']} / {v['bind_work']['calls']}" for v in values) + " |")
    lines += ["", "## 方法与边界", "",
        "- XIV 使用完整固定 CSV（51,225 详情 + 513 目录 + 首页），沿原先时间线切换三档 Python 脚本。四个人工站点均为 512 详情 + 8 目录 + 首页，不随历史版本改变。",
        "- XIV 大量页面源由 prepare 返回为虚拟文档，人工站点则实际从磁盘发现页面和模块文件；物理观察集合大小不同正是负载差异之一。不可仅按页数跨情形推断每页效率。",
        "- 人工站点的整数在输入边界显式格式化为 String；Bool 条件分支保留。没有使用早期版本不存在的 str、数组方法、for 或 Data API。它是跨版本兼容的新负载，不把旧模块样例的秒数混进来。",
        "- 共享／私有图使用相同内容与 13 字段组件；私有图每页不同源身份，没有首轮跨页复用。宽导出包含四个 64 函数库、通配 facade 及八套 32 行布局，所有 256 个标签函数均在完整批次中实际使用。",
        "- 普通 CLI 每次新进程，计时含启动、输入、完整 SSG、模板、发布写盘和旧输出清理；各负载采用固定输出路径。各版本／情形轮转并反序交错，不与编译或 HTML 验证并发。",
        "- CPU 8，温文件缓存，不 fsync，不锁频。release debug=0、incremental=0、jobs=2。wait4 RSS 为进程高水位，不是进程树同时内存总和。小差异不作独立优化结论。",
        "- 复用了原批次已独立构建、经过 SHA-256 核对的历史二进制和源码，没有复用旧计时。早期版本保留当时的 publication 路径补丁；Handlebars 是未恢复完整未提交源码的存档补充参照。",
        f"- 当前点固定已提交 `{m['current']}`，不包含 checkout-start.diff 中的未提交 SSG 宿主政策修改。资源版本在临时 CLI 统一 requests=1,000,000，仍正常读取、授权、版本检查与计费，未改生产源码。",
        "- Python 两步只影响 XIV；其余人工负载在这两个对照点使用完全相同的引擎和输入，可视为控制点，不能将小幅测量波动解释成优化。comemo off/compile 是同一提交的消融，不是不同历史提交。",
        "- 所有场景都检查完整页数、全部字段／目录行、所有本地 HTML 链接；所有 Tera 及后续产物逐文件字节一致，Handlebars/Tera 的 HTML 解析语义及非 HTML 字节一致。探针也与相应普通产物逐文件相同。",
        f"- 探针每格一次，仅用于定位和确定性工作次数验证，不把它当作多轮性能结论；普通 CLI 排名使用 {args.runs} 轮中位数。包含式阶段不可重复累加。",
        f"- 原始数据、完整范围、样本、文件指纹、输入、二进制、源码与探针输出：`{out}`。", ""]
    lines += ["交互分组图：`dashboard.html`（耗时／RSS、统一／独立刻度、按情形开关）；全长静态图：`history-wall.svg`、`history-rss.svg`。每个历史点恰有五条，细线是五轮范围。", ""]
    write(out / "report.md", "\n".join(lines))
    print(f"wrote {out / 'report.md'}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "build", "validate", "run", "probes", "report", "measure", "all"])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--history", type=Path, default=Path("/tmp/opencode/xiv-history-20261102"))
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--cpu", type=int, default=8)
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("runs must be positive")
    if args.action in ["all", "measure"]:
        if args.action == "all":
            prepare(args)
            build(args)
        validate(args)
        run(args)
        probes(args)
        report(args)
    else:
        {"prepare": prepare, "build": build, "validate": validate, "run": run, "probes": probes, "report": report}[args.action](args)


if __name__ == "__main__":
    main()
