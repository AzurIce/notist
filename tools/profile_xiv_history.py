#!/usr/bin/env python3
"""Re-run the XIV DB optimization history in disposable, fingerprinted snapshots.

Preparation/build and validation are outside measured regions. No consumer or
production default is edited. See the generated report for compatibility patches.
"""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile

REPO = Path(__file__).resolve().parents[1]
CSV_SHA = "abb68a057f68e4d9d37eae76d31354555d30f76a88554d0d1c04315598c7268f"
ENGINES = [
    ("tera", "debe71b", "Tera 2 迁移"),
    ("renderer", "a03620f", "Renderer 直接构造"),
    ("html-shared", "1111fe7", "共享 HTML targets"),
    ("borrowed", "d7d652b", "去整站 JSON 中间树"),
    ("context", "86451bd", "共享页面／目录 Tera 值"),
    ("semantic", "8401ea8", "共享语义定义"),
    ("environment", "0d81985", "复用环境句柄"),
    ("resources", "ef38d1b", "资源／共享值／Data 重构"),
    ("modules", "268fb35", "拆分编译与导入绑定"),
    ("listen", "79d4734", "CLI listen 重构（build 控制）"),
    ("memo-off", "daed34c", "comemo 代码：关闭两处缓存"),
    ("memo-compile", "daed34c", "comemo：仅编译缓存"),
    ("memo-both", "daed34c", "comemo：编译＋绑定缓存"),
    ("head", "2468767", "移除模板 Markdown helper"),
    ("working", "WORKING", "当前未提交语言能力"),
]
CASES = [
    ("handlebars", "handlebars", "a76ae04", "存档 Handlebars 优化基线"),
    ("tera", "tera", "a76ae04", "Tera 2"),
    ("renderer", "renderer", "a76ae04", "直接 Renderer"),
    ("html-shared", "html-shared", "a76ae04", "共享 HTML targets"),
    ("borrowed", "borrowed", "a76ae04", "去中间 JSON"),
    ("context", "context", "a76ae04", "共享 Tera 值"),
    ("json-batch", "context", "f4fc8d5", "Python 批量紧凑编码"),
    ("catalog", "context", "1832d47", "目录四字段摘要"),
    ("semantic", "semantic", "1832d47", "共享语义定义"),
    ("environment", "environment", "1832d47", "复用环境句柄"),
    ("resources", "resources", "1832d47", "资源／共享值／Data"),
    ("modules", "modules", "1832d47", "模块分层"),
    ("listen", "listen", "1832d47", "CLI listen 控制"),
    ("memo-off", "memo-off", "1832d47", "comemo ablation：off"),
    ("memo-compile", "memo-compile", "1832d47", "comemo ablation：compile"),
    ("memo-both", "memo-both", "1832d47", "comemo：both"),
    ("head", "head", "1832d47", "2468767：移除模板 Markdown helper"),
    ("working", "working", "1832d47", "工作区：str／len／for"),
]


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def git(repo, *args):
    return subprocess.check_output(["git", "-C", str(repo), *args])


def save(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n")


def extract(repo, rev, dest):
    dest.mkdir(parents=True)
    raw = git(repo, "archive", rev)
    with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
        archive.extractall(dest, filter="data")


def once(text, old, new):
    if text.count(old) != 1:
        raise ValueError(f"expected one match, got {text.count(old)}: {old[:100]}")
    return text.replace(old, new, 1)


def prepare(args):
    out = args.out.resolve()
    if out.exists():
        raise ValueError("use a new result directory")
    for consumer in [REPO.resolve(), args.xiv.resolve()]:
        if out == consumer or consumer in out.parents:
            raise ValueError("result directory must be outside the source checkouts")
    out.mkdir(parents=True)
    (out / "sources").mkdir()
    (out / "bin").mkdir()
    (out / "logs").mkdir()
    manifest = {"engines": {}, "cases": CASES, "head": git(REPO, "rev-parse", "HEAD").decode().strip(),
                "status": git(REPO, "status", "--porcelain=v1").decode(), "csv_sha256": CSV_SHA,
                "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
                "build_env": {"CARGO_PROFILE_RELEASE_DEBUG": "0", "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "2"},
                "site_source": str(args.site_source.resolve()), "xiv": str(args.xiv.resolve())}
    (out / "start.diff").write_bytes(git(REPO, "diff", "--binary", "HEAD"))
    (out / "publication.patch").write_bytes(args.publication_patch.read_bytes())
    for name, revision, label in ENGINES:
        dest = out / "sources" / name
        rev = manifest["head"] if revision == "WORKING" else git(REPO, "rev-parse", revision).decode().strip()
        extract(REPO, rev, dest)
        if revision == "WORKING":
            changed = git(REPO, "diff", "--name-only", "HEAD").decode().splitlines()
            changed += git(REPO, "ls-files", "--others", "--exclude-standard").decode().splitlines()
            for file in changed:
                source, target = REPO / file, dest / file
                if source.is_file():
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, target)
                elif target.is_file():
                    target.unlink()
        adaptations = []
        publication = dest / "src/publication/mod.rs"
        if ".keys()\n            .any(|previous|" in publication.read_text():
            subprocess.run(["git", "apply", str(out / "publication.patch")], cwd=dest, check=True)
            adaptations.append("historical preexisting publication-path patch")
        cli = dest / "crates/notist-cli/src/ssg.rs"
        if (dest / "src/resource_session.rs").exists():
            text = once(cli.read_text(),
                        "Vault::new(prepared.files).with_config(prepared.config)",
                        'Vault::new(prepared.files).with_input_limits(notist::InputLimits { requests: std::env::var("XIV_PROFILE_REQUESTS").map(|v| v.parse().unwrap()).unwrap_or(16384), ..Default::default() }).with_config(prepared.config)')
            cli.write_text(text)
            adaptations.append("benchmark-only environment override for request budget; default remains 16384")
        if name in ["memo-off", "memo-compile"]:
            names = ["modules.rs"] + (["definitions.rs"] if name == "memo-off" else [])
            for file in names:
                path = dest / "crates/notist-pipeline/src" / file
                text = once(path.read_text(), '#[cfg(not(feature = "profiling"))]\n    {\n        true\n    }',
                            '#[cfg(not(feature = "profiling"))]\n    {\n        false\n    }')
                path.write_text(text)
            adaptations.append("cache ablation: " + name)
        manifest["engines"][name] = {"revision": rev, "label": label, "source": str(dest), "adaptations": adaptations}
    # The historical pre-migration working tree was not committed. Preserve the
    # audited original artifact; do not pretend it is a clean 574db84 checkout.
    binary = out / "bin/handlebars"
    shutil.copy2(args.handlebars_bin, binary)
    expected = "afe6a72b6a097638d40f48d81c1a6f54372ef188d9eb14a80199fdbba85e60e0"
    if sha(binary.read_bytes()) != expected:
        raise ValueError("archived Handlebars artifact differs from its recorded fingerprint")
    manifest["engines"]["handlebars"] = {"revision": "historical working tree before debe71b", "binary_sha256": expected,
        "label": "存档 Handlebars，非干净 commit", "source": None,
        "adaptations": ["archived context-reuse/publication-patch build; see historical report"]}
    for directory in ["content", "data", "static", "sources", "inputs"]:
        shutil.copytree(args.site_source / directory, out / "site" / directory)
    shutil.copy2(args.site_source / "Notist.toml", out / "site/Notist.toml")
    shutil.copytree(args.site_source / "templates", out / "templates-tera")
    shutil.copytree(args.handlebars_templates, out / "templates-handlebars")
    (out / "site/scripts").mkdir()
    shutil.copy2(args.xiv / "scripts/check_site.py", out / "site/scripts/check_site.py")
    (out / "scripts").mkdir()
    for rev in ["a76ae04", "f4fc8d5", "1832d47"]:
        (out / "scripts" / f"{rev}.py").write_bytes(git(args.xiv, "show", f"{rev}:scripts/items.py"))
    if sha((out / "site/inputs/Item.csv").read_bytes()) != CSV_SHA:
        raise ValueError("wrong CSV")
    paths = {}
    for p in sorted((out / "site").rglob("*")):
        if p.is_file():
            paths[str(p.relative_to(out / "site"))] = sha(p.read_bytes())
    manifest["fixed_site_files"] = paths
    save(out / "manifest.json", manifest)
    print(f"prepared {len(ENGINES)} source snapshots and {len(CASES)} full-site checkpoints", flush=True)


def build(args):
    out = args.out.resolve()
    manifest = json.loads((out / "manifest.json").read_text())
    for name, revision, label in ENGINES:
        binary = out / "bin" / name
        target = out / "targets" / name
        if binary.exists() and manifest["engines"][name].get("target_dir") == str(target):
            continue
        env = dict(os.environ, CARGO_TARGET_DIR=str(target), **manifest["build_env"], NO_COLOR="1", TERM="dumb")
        source = Path(manifest["engines"][name]["source"])
        command = ["cargo", "build", "--locked", "--release", "-p", "notist-cli"]
        with (out / "logs" / f"build-{name}.log").open("wb") as log:
            subprocess.run(command, cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
        shutil.copy2(target / "release/notist", binary)
        manifest["engines"][name]["binary_sha256"] = sha(binary.read_bytes())
        manifest["engines"][name]["target_dir"] = str(target)
        save(out / "manifest.json", manifest)
        print(f"built {name}: {revision}", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["prepare", "build", "measure", "profile-build", "run", "profiles", "report"])
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--xiv", type=Path, default=Path("/home/azurice/Files/_ffxiv/icexiv-db"))
    parser.add_argument("--site-source", type=Path, default=Path("/tmp/opencode/xiv-db-prepare-environment/site"))
    parser.add_argument("--publication-patch", type=Path, default=Path("/tmp/opencode/xiv-db-registry-context/preexisting-publication.patch"))
    parser.add_argument("--handlebars-bin", type=Path, default=Path("/tmp/opencode/xiv-db-tera-comparison/notist-handlebars"))
    parser.add_argument("--handlebars-templates", type=Path, default=Path("/tmp/opencode/xiv-db-tera-comparison/templates-handlebars"))
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--cpu", type=int, default=8)
    args = parser.parse_args()
    if args.action == "prepare":
        prepare(args)
    elif args.action == "build":
        build(args)
    else:
        from xiv_history_probes import dispatch
        dispatch(args)


if __name__ == "__main__":
    main()
