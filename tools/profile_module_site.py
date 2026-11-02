#!/usr/bin/env python3
"""A complete module-rich SSG workload, cache invalidation, and reuse controls.

Fictional deterministic records, not a replacement benchmark for production XIV
DB. No production defaults, consumers, locks, or engine implementation are edited.
"""
import argparse
import hashlib
from html.parser import HTMLParser
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time
from urllib.parse import unquote, urlsplit

REPO = Path(__file__).resolve().parents[1]
FIELDS = {"id": "Int", "name": "String", "description": "String", "category": "Int", "searchCategory": "Int",
          "equipLevel": "Int", "rarity": "Int", "stackSize": "Int", "icon": "Int",
          "unique": "Bool", "untradable": "Bool", "hq": "Bool", "path": "String"}
MODES = ["off", "compile", "both"]


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value)


def literal(value):
    return json.dumps(value, ensure_ascii=False)


def modules():
    parameters = ", ".join(f"{k}: {t}" for k, t in FIELDS.items())
    arguments = ", ".join(FIELDS)
    rows = []
    for key, kind in FIELDS.items():
        expression = f"flag({key})" if kind == "Bool" else f"number({key})" if kind == "Int" else f"describe_text({key})" if key == "description" else f"item_name({key})"
        rows.append(f'#row[#cell[#text(text: label_{key}())]#cell[#text(text: {expression})]]')
    library = {
        "version.notc": 'fn stamp() -> String { "stamp-A" }\n',
        "variant.notc": 'fn stamp() -> String { "stamp-C" }\n',
        "labels.notc": "".join(f'fn label_{k}() -> String {{ "{k}" }}\n' for k in FIELDS),
        "numbers.notc": 'fn number(value: Int) -> String { str(value) }\n',
        "flags.notc": 'fn flag(value: Bool) -> String { if value { "是" } else { "否" } }\n',
        "names.notc": 'fn item_name(value: String) -> String { value }\nfn describe_text(value: String) -> String { value }\n',
        "metadata.notc": 'fn kind() -> String { "静态物品目录" }\nfn schema() -> String { "schema-1" }\n',
        "public.notc": "".join(f'import "{name}.notc": *;\n' for name in ["labels", "numbers", "flags", "names", "metadata", "version"]),
        "badges.notc": 'import "version.notc": stamp;\nfn badge() -> InlineContent { [徽标：#text(text: stamp())] }\n',
        "detail.notc": 'import "public.notc": *;\nimport "badges.notc": badge;\n' +
            f'fn detail({parameters}) -> BlockContent {{\n[\n#paragraph[版本：#text(text: stamp()) · #badge() · #text(text: kind()) · #text(text: schema())]\n\n#table()[\n' +
            "\n".join(rows) + '\n]\n]\n}\n',
        "catalog.notc": 'import "public.notc": stamp, kind;\nimport "badges.notc": badge;\nfn intro(title: String) -> BlockContent {\n[\n#paragraph[#text(text: title) · #text(text: kind()) · #text(text: stamp()) · #badge()]\n]\n}\n',
    }
    for group in range(8):
        library[f"section{group}.notc"] = 'import "detail.notc": detail;\n' + f'fn section_name() -> String {{ "group-{group}-A" }}\nfn show({parameters}) -> BlockContent {{\n[\n#paragraph[#text(text: section_name())]\n\n#detail({arguments})\n]\n}}\n'
    return library


def record(index):
    item_id = 10001 + index
    return {"id": item_id, "name": f"示例物品 {index:04d}" + "★" * (index % 5),
            "description": f"第 {index} 件物品，包含 <tag> & 引号 \"，用于离线展示。",
            "category": index % 23, "searchCategory": index % 17, "equipLevel": index % 100,
            "rarity": index % 5, "stackSize": 99 if index % 2 else 999, "icon": 100000 + index,
            "unique": index % 7 == 0, "untradable": index % 3 == 0, "hq": index % 2 == 0,
            "path": f"items/{item_id}.html"}


def direct_body(item, group):
    rows = []
    for key, kind in FIELDS.items():
        # Standalone Markup constructor arguments are data literals, not Code.
        expression = literal("是" if item[key] is True else "否" if item[key] is False else str(item[key]))
        rows.append(f'#row[#cell[{key}]#cell[#text(text: {expression})]]')
    return f'#paragraph[group-{group}-A]\n\n#paragraph[版本：stamp-A · 徽标：stamp-A · 静态物品目录 · schema-1]\n\n#table()[\n' + "\n".join(rows) + "\n]\n"


def fixture(dest, pages, reuse):
    site = dest / "site"
    library = modules()
    records = [record(i) for i in range(pages)]
    # Nothing is fetched. All data is represented by ordinary page arguments;
    # the module caches do not memoize data reads, documents, evaluation, or HTML.
    write(site / "Notist.toml", "extensions = []\n")
    write(site / "templates/page.tera", '<!doctype html><html lang="zh-CN"><head><meta charset="utf-8"><title>{{ page.title }} · {{ site.title }}</title><link rel="stylesheet" href="/style.css"></head><body><nav><a href="/index.html">首页</a> <a href="/catalog/index.html">目录</a></nav><main>{{ page.html | safe }}</main></body></html>')
    write(site / "static/style.css", "body{font-family:sans-serif;max-width:60rem;margin:auto}td{padding:.3rem}nav{padding:1rem}\n")
    if reuse != "none":
        for name, text in library.items():
            write(site / "modules" / name, text)
    for index, item in enumerate(records):
        group, form = index % 8, index % 3
        source = f'@!(title: {literal(item["name"])})\n'
        prefix = "../../modules"
        if reuse == "private":
            prefix = f"../../private/{item['id']}"
            for name, text in library.items():
                write(site / "private" / str(item["id"]) / name, text)
        if reuse == "none":
            source += direct_body(item, group)
        else:
            target = f"{prefix}/section{group}.notc"
            if form == 0:
                source += f'#import "{target}" as view;\n'
                call = "view.show"
            else:
                source += f'#import "{target}": {"show" if form == 1 else "*"};\n'
                call = "show"
            source += "#" + call + "(" + ", ".join(literal(item[k]) for k in FIELDS) + ")\n"
        source += '\n#paragraph[#link("../catalog/index.not")[返回目录]]\n'
        write(site / "content" / "items" / f"{item['id']}.not", source)
    catalog_count = (pages + 63) // 64
    for index in range(catalog_count):
        source = f'@!(title: "目录 {index + 1}")\n'
        source += '#import "../../modules/catalog.notc" as catalog;\n#catalog.intro("商品目录")\n\n' if reuse == "shared" else '#paragraph[商品目录]\n\n'
        source += '#table()[\n' + "\n".join(f'#row[#cell[{r["id"]}]#cell[#link("../items/{r["id"]}.not")[#text(text: {literal(r["name"])})]]]' for r in records[index * 64:(index + 1) * 64]) + '\n]\n'
        if index:
            source += f'\n#paragraph[#link("{index}.not")[上一页]]\n' if index > 1 else '\n#paragraph[#link("index.not")[上一页]]\n'
        if index + 1 < catalog_count:
            source += f'\n#paragraph[#link("{index + 2}.not")[下一页]]\n'
        write(site / "content/catalog" / ("index.not" if index == 0 else f"{index + 1}.not"), source)
    write(site / "content/index.not", '@!(title: "模块缓存测试站")\n#paragraph[完整的离线示例商品站点。]\n\n#paragraph[#link("catalog/index.not")[浏览所有物品]]\n')
    scenarios = [{"name": "cold", "overrides": {}, "error": False}, {"name": "warm", "overrides": {}, "error": False}]
    if reuse == "shared":
        body = library["version.notc"].replace("stamp-A", "stamp-B")
        base = {"modules/version.notc": body}
        scenarios += [
            {"name": "body", "overrides": base, "error": False},
            {"name": "export", "overrides": {**base, "modules/version.notc": body + 'fn new_badge() -> String { "unused-export" }\n'}, "error": False},
            {"name": "local", "overrides": {**base, "modules/section0.notc": library["section0.notc"].replace("group-0-A", "group-0-B")}, "error": False},
            {"name": "redirect", "overrides": {**base, "modules/public.notc": library["public.notc"].replace("version.notc", "variant.notc")}, "error": False},
            {"name": "missing", "overrides": {"modules/version.notc": body.replace("fn stamp()", "fn stump()")}, "error": True},
            {"name": "missing-warm", "overrides": {"modules/version.notc": body.replace("fn stamp()", "fn stump()")}, "error": True},
            {"name": "repaired", "overrides": base, "error": False},
            {"name": "evicted", "overrides": base, "error": False},
        ]
    config = {"title": "模块缓存测试站", "content": "content", "templates": "templates", "static_dir": "static", "output": "dist", "template": "page"}
    save(dest / "fixture.json", {"reuse": reuse, "detail_count": pages, "catalog_count": catalog_count, "page_count": pages + catalog_count + 1, "records": records, "config": config, "scenarios": scenarios})


def prepare(args):
    out = args.out.resolve()
    if out.exists():
        raise ValueError("use a new output directory")
    out.mkdir(parents=True)
    for name, pages, reuse in [("shared", args.pages, "shared"), ("private", args.private_pages, "private"), ("no-imports", args.pages, "none")]:
        fixture(out / "fixtures" / name, pages, reuse)
    hashes = {str(p.relative_to(out)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted((out / "fixtures").rglob("*")) if p.is_file()}
    save(out / "manifest.json", {"fixture_files": hashes, "pages": args.pages, "private_pages": args.private_pages,
         "head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
         "rustc": subprocess.check_output(["rustc", "-Vv"], text=True), "python": platform.python_version(), "system": platform.platform()})
    (out / "source.diff").write_bytes(subprocess.check_output(["git", "diff", "--binary", "HEAD"], cwd=REPO))
    (out / "logs").mkdir()


def build(args):
    out = args.out.resolve()
    env = dict(os.environ, CARGO_TARGET_DIR=str(out / "target"), CARGO_BUILD_JOBS="2", CARGO_INCREMENTAL="0", CARGO_PROFILE_RELEASE_DEBUG="0", NO_COLOR="1", TERM="dumb")
    with (out / "logs/build.log").open("wb") as log:
        subprocess.run(["cargo", "build", "--locked", "--release", "-p", "notist-cli", "--features", "profiling", "--example", "module_site_profile"], cwd=REPO, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    binary = out / "target/release/examples/module_site_profile"
    save(out / "binary.json", {"path": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest()})


class Page(HTMLParser):
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.links, self.rows, self.text = [], [], []
        self.row = self.cell = None

    def handle_starttag(self, tag, attrs):
        self.links += [v for k, v in attrs if k in ["href", "src"] and v]
        if tag == "tr": self.row = []
        if tag in ["td", "th"]: self.cell = []

    def handle_endtag(self, tag):
        if tag in ["td", "th"] and self.cell is not None:
            self.row.append("".join(self.cell)); self.cell = None
        if tag == "tr" and self.row is not None:
            self.rows.append(self.row); self.row = None

    def handle_data(self, text):
        self.text.append(text)
        if self.cell is not None: self.cell.append(text)


def verify(output, spec, scenario):
    if scenario["error"]:
        errors = json.loads((output / "errors.json").read_text())
        assert any("has no public member `stamp`" in e["message"] for e in errors)
        return {"errors_sha256": hashlib.sha256((output / "errors.json").read_bytes()).hexdigest(), "errors": len(errors)}
    paths, html_count = {}, 0
    records = {f"items/{r['id']}.html": (i, r) for i, r in enumerate(spec["records"])}
    catalogs = []
    public_stamp = "stamp-C" if scenario["name"] == "redirect" else "stamp-A" if scenario["name"] in ["cold", "warm"] else "stamp-B"
    for path in sorted(output.rglob("*")):
        if not path.is_file(): continue
        rel = path.relative_to(output).as_posix()
        paths[rel] = hashlib.sha256(path.read_bytes()).hexdigest()
        if path.suffix != ".html": continue
        html_count += 1
        parser = Page(); parser.feed(path.read_text()); parser.close()
        for link in parser.links:
            url = urlsplit(link)
            if url.scheme or url.netloc or not url.path: continue
            target = (output if url.path.startswith("/") else path.parent) / unquote(url.path.lstrip("/"))
            assert target.exists(), (path, link)
        if rel in records:
            index, item = records[rel]
            assert len(parser.rows) == len(FIELDS), (rel, parser.rows)
            expected = {k: "是" if v is True else "否" if v is False else str(v) for k, v in item.items()}
            assert dict(parser.rows) == expected, (rel, parser.rows, expected)
            text = "".join(parser.text)
            group = index % 8
            suffix = "B" if scenario["name"] == "local" and group == 0 else "A"
            assert f"group-{group}-{suffix}" in text, (rel, scenario["name"])
            assert f"版本：{public_stamp}" in text, (rel, scenario["name"], text)
            assert f"徽标：{'stamp-A' if scenario['name'] in ['cold','warm'] else 'stamp-B'}" in text
        elif rel.startswith("catalog/"):
            catalogs.extend(int(r[0]) for r in parser.rows)
    assert html_count == spec["page_count"]
    assert sorted(catalogs) == [r["id"] for r in spec["records"]]
    assert {p for p in paths if p.startswith("items/")} == set(records)
    digest = hashlib.sha256()
    for path, value in paths.items(): digest.update(path.encode() + b"\0" + bytes.fromhex(value))
    return {"sha256": digest.hexdigest(), "files": len(paths), "html": html_count, "paths": paths}


def verify_cache_work(sample):
    steps = {s["scenario"]: s for s in sample["steps"]}
    work = lambda step, stage: steps[step]["stages"][stage + "_work"]["calls"]
    if sample["workload"] == "no-imports":
        assert all(work(step, "compile") == 0 for step in steps)
    if sample["mode"] == "off":
        for step in steps.values():
            assert step["stages"]["compile_work"]["calls"] == step["stages"]["compile"]["calls"]
            assert step["stages"]["bind_work"]["calls"] == step["stages"]["bind"]["calls"]
    else:
        assert work("warm", "compile") == 0
        if sample["workload"] == "shared":
            assert all(work(step, "compile") == 1 for step in ["body", "export", "local", "missing"])
            assert work("redirect", "compile") == 2
            assert work("missing-warm", "compile") == work("repaired", "compile") == 0
            assert work("evicted", "compile") == work("cold", "compile") > 0
        if sample["mode"] == "both":
            assert work("warm", "bind") == 0
            if sample["workload"] == "shared":
                assert all(work(step, "bind") == 0 for step in ["body", "local", "missing-warm", "repaired"])
                assert work("export", "bind") > 0 and work("redirect", "bind") > 0
                assert work("evicted", "bind") == work("cold", "bind") > 0


def run(args):
    out = args.out.resolve()
    manifest = json.loads((out / "manifest.json").read_text())
    for rel, expected in manifest["fixture_files"].items():
        assert hashlib.sha256((out / rel).read_bytes()).hexdigest() == expected, rel
    binary = json.loads((out / "binary.json").read_text())
    assert hashlib.sha256(Path(binary["path"]).read_bytes()).hexdigest() == binary["sha256"]
    result_path = out / "runs.json"
    result = json.loads(result_path.read_text()) if result_path.exists() else {"samples": [], "runs": args.runs, "cpu": args.cpu}
    assert result["runs"] == args.runs and result["cpu"] == args.cpu
    for rep in range(args.runs):
        order = MODES[rep % 3:] + MODES[:rep % 3]
        for workload in ["shared", "private", "no-imports"]:
            for mode in order:
                if any(s["rep"] == rep and s["workload"] == workload and s["mode"] == mode for s in result["samples"]): continue
                destination = out / "outputs" / f"{workload}-{mode}-{rep}"
                if destination.exists(): raise ValueError(f"incomplete run directory: {destination}; choose a new output root")
                command = ["taskset", "-c", str(args.cpu), binary["path"], str(out / "fixtures" / workload), mode, str(destination)]
                log = out / "logs" / f"{workload}-{mode}-{rep}"
                start = time.perf_counter()
                with log.with_suffix(".stdout").open("wb") as stdout, log.with_suffix(".stderr").open("wb") as stderr:
                    child = subprocess.Popen(command, cwd=REPO, env=dict(os.environ, NO_COLOR="1", TERM="dumb"), stdout=stdout, stderr=stderr)
                    _, status, usage = os.wait4(child.pid, 0)
                    child.returncode = os.waitstatus_to_exitcode(status)
                if child.returncode:
                    raise RuntimeError(log.with_suffix(".stderr").read_text()[-12000:])
                sample = {"workload": workload, "mode": mode, "rep": rep, "command": command, "process_wall_s": time.perf_counter() - start,
                          "user_s": usage.ru_utime, "system_s": usage.ru_stime, "peak_rss_kib": usage.ru_maxrss,
                          "steps": [json.loads(line) for line in log.with_suffix(".stdout").read_text().splitlines()]}
                result["samples"].append(sample)
                save(result_path, result)
                print(f"measured {workload} {mode} {rep + 1}/{args.runs}", flush=True)
    # Expensive HTML validation/hashing runs only after all timed workers exit.
    references = {}
    for sample in result["samples"]:
        verify_cache_work(sample)
        spec = json.loads((out / "fixtures" / sample["workload"] / "fixture.json").read_text())
        values = {}
        for step, scenario in zip(sample["steps"], spec["scenarios"], strict=True):
            assert step["scenario"] == scenario["name"]
            verified = verify(Path(step["output"]), spec, scenario)
            key = sample["workload"], scenario["name"]
            if key in references: assert verified == references[key], (key, sample["mode"])
            else: references[key] = verified
            values[scenario["name"]] = verified
        assert values["cold"] == values["warm"]
        if sample["workload"] == "shared":
            assert values["body"] == values["export"] == values["repaired"] == values["evicted"]
            assert values["cold"] != values["body"] != values["redirect"]
            assert values["missing"] == values["missing-warm"]
            changed = [p for p, digest in values["local"]["paths"].items() if digest != values["body"]["paths"][p]]
            expected = [f"items/{r['id']}.html" for i, r in enumerate(spec["records"]) if i % 8 == 0]
            assert set(changed) == set(expected)
        sample["verified"] = {name: {k: v for k, v in data.items() if k != "paths"} for name, data in values.items()}
    result["validated"] = True
    save(result_path, result)


def report(args):
    out = args.out.resolve()
    result = json.loads((out / "runs.json").read_text())
    assert result.get("validated")
    summary = {"workloads": {}, "runs": result["runs"], "cpu": result["cpu"]}
    for workload in ["shared", "private", "no-imports"]:
        data = {}
        for mode in MODES:
            runs = [s for s in result["samples"] if s["workload"] == workload and s["mode"] == mode]
            steps = {}
            for name in [s["scenario"] for s in runs[0]["steps"]]:
                samples = [next(s for s in r["steps"] if s["scenario"] == name) for r in runs]
                steps[name] = {"ms": statistics.median(s["assembly_ms"] for s in samples), "min_ms": min(s["assembly_ms"] for s in samples), "max_ms": max(s["assembly_ms"] for s in samples),
                    "stages": {stage: {field: statistics.median(s["stages"][stage][field] for s in samples) for field in ["calls", "ms"]} for stage in samples[0]["stages"]}}
            data[mode] = {"steps": steps, "rss_mib": statistics.median(r["peak_rss_kib"] for r in runs) / 1024}
        summary["workloads"][workload] = data
    save(out / "summary.json", summary)
    lines = ["# 模块密集型完整站点：comemo 评估", "", "这是另行构造的虚构商品站点，不是生产 XIV DB 的加速比。每轮新建 Vault，完整执行 SSG 分析、求值、HTML、模板及内存产物组装；缓存只覆盖模块编译和导入绑定。", ""]
    shared = summary["workloads"]["shared"]
    off, both = shared["off"], shared["both"]
    lines += ["## 结论", "",
        f"- 共享组件站点首轮 {off['steps']['cold']['ms']:.2f} → {both['steps']['cold']['ms']:.2f} ms，减少 {(1 - both['steps']['cold']['ms'] / off['steps']['cold']['ms']) * 100:.1f}%；同进程不变重建 {off['steps']['warm']['ms']:.2f} → {both['steps']['warm']['ms']:.2f} ms，减少 {(1 - both['steps']['warm']['ms'] / off['steps']['warm']['ms']) * 100:.1f}%。",
        f"- 进程峰值 RSS {off['rss_mib']:.1f} → {both['rss_mib']:.1f} MiB，减少 {(1 - both['rss_mib'] / off['rss_mib']) * 100:.1f}%。",
        "- 首轮已经能跨页复用，不必等到第二次整站重建。编译缓存贡献主要的整站收益；绑定缓存消除了真实绑定重算，但相对于 compile-only 的额外整站耗时收益小、范围重叠，本轮不作稳定加速结论。",
        "- 未缓存页面解析、实例检查、求值、资源读取或模板。模块编译与绑定自身的工作大幅减少，不等于整站能得到同样倍数的加速。",
        "- 私有模块图首轮没有明显收益，后续重建才出现复用；无导入对照没有稳定收益。这支持按真实复用机会解释缓存，而不是宣称所有站点都会更快。", ""]
    for workload, data in summary["workloads"].items():
        spec = json.loads((out / "fixtures" / workload / "fixture.json").read_text())
        lines += [f"## {workload}：{spec['detail_count']} 详情、{spec['catalog_count']} 目录、首页，共 {spec['page_count']} 页", "", "| 场景 | off ms | compile ms | both ms | off 编译/绑定执行 | compile 编译/绑定执行 | both 编译/绑定执行 |", "| --- | ---: | ---: | ---: | ---: | ---: | ---: |"]
        for name in data["off"]["steps"]:
            steps = [data[mode]["steps"][name] for mode in MODES]
            counts = [f"{s['stages']['compile_work']['calls']:.0f}/{s['stages']['bind_work']['calls']:.0f}" for s in steps]
            lines.append(f"| {name} | " + " | ".join(f"{s['ms']:.2f}" for s in steps) + " | " + " | ".join(counts) + " |")
        lines += ["", "进程峰值 RSS（覆盖整个编辑序列，并非每轮独立峰值）：" + "，".join(f"{mode} {data[mode]['rss_mib']:.1f} MiB" for mode in MODES) + "。", ""]
        lines += ["首轮／不变重建的实测范围（ms）：" + "；".join(
            f"{mode} " + " / ".join(f"{data[mode]['steps'][name]['min_ms']:.2f}–{data[mode]['steps'][name]['max_ms']:.2f}" for name in ["cold", "warm"]) for mode in MODES) + "。", ""]
    lines += ["## 方法与边界", "", f"- 同一 release 二进制切换三个 profiling-only 模式，各 {args.runs} 个独立进程，轮转顺序，CPU {args.cpu}。首轮进程级计算缓存为空；后续保留缓存但新建 Vault。",
              "- shared 使用多层共享组件库，包含命名空间、成员选择、通配导入和重导出；每个字段和模块输出实际参与页面。private 为每页独立模块图，限制首轮跨页复用；no-imports 是无模块导入对照。",
              "- body 修改同长度函数体；export 在末尾增加未使用导出；local 只改第 0 组页面；redirect 更改传递重导出目标；missing/missing-warm 删除必需导出并重复；repaired 恢复正确源码；evicted 显式清空缓存后重建。",
              "- 失败场景只验证诊断与错误缓存，不把快速失败算成完整构建加速。",
              "- 输入预先装入不可变 MemoryResources，编辑快照制作在计时之外；每次仍经过正常 Vault 读取、授权、版本与计费。统一显式 requests=1,000,000，不改变生产默认值。",
              "- 计时包含批次缓存淘汰、Vault 新建、SSG 全量组装和 Vault 释放；不含 fixture 生成、Rust 编译、验证副本写盘、输出对象释放、HTML 解析和 SHA-256。不是 CLI 发布事务或磁盘 benchmark。",
              "- 各模式／重复的每个场景逐文件 SHA-256 相同；全部字段、物品集、目录行和本地链接通过验证。验证同长度正文更新确实更新页面、未使用导出不改变 HTML、局部修改只影响目标组、重导出目标切换生效、错误后恢复正确。",
              "- compile_work 是 compile 的子阶段，bind 是 link 的子阶段；包含式时间不可全部相加。RSS 为 wait4 高水位，不是同时进程树内存总和；频率未锁定。",
              f"- 原始输入、逐次 JSON、输出、stdout/stderr、二进制及指纹保存在 `{out}`。", ""]
    write(out / "report.md", "\n".join(lines))
    print(f"wrote {out / 'report.md'}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "build", "run", "report", "all"])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--pages", type=int, default=512)
    parser.add_argument("--private-pages", type=int, default=64)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--cpu", type=int, default=8)
    args = parser.parse_args()
    if min(args.pages, args.private_pages, args.runs) < 1: parser.error("counts must be positive")
    if args.action == "all":
        prepare(args); build(args); run(args); report(args)
    else:
        {"prepare": prepare, "build": build, "run": run, "report": report}[args.action](args)


if __name__ == "__main__":
    main()
