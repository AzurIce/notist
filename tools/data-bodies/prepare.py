#!/usr/bin/env python3
"""Read-only consumer snapshots for the standalone notc acceptance host. No network."""
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import tomllib


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--xiv", type=Path, required=True)
    parser.add_argument("--ranim", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    out = args.out.resolve()
    for consumer in [args.xiv.resolve(), args.ranim.resolve()]:
        if out == consumer or consumer in out.parents:
            parser.error("fixture output must be outside the consumer checkouts")
    if out.exists() and any(out.iterdir()):
        parser.error("use a new empty fixture directory")
    out.mkdir(parents=True, exist_ok=True)
    versions = {}

    def read(path):
        raw = path.read_bytes()
        versions[str(path.resolve())] = digest(raw)
        return raw

    def write(name, value):
        path = out / name
        if out not in path.resolve().parents:
            raise ValueError(f"fixture path escapes output: {name}")
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value, ensure_ascii=False, separators=(",", ":")))

    script = args.xiv / "scripts/items.py"
    read(script)
    # run_path does not import/cache a module in the read-only consumer tree.
    adapter = runpy.run_path(str(script))
    source = json.loads(read(args.xiv / "sources/item.json"))
    csv = read(args.xiv / "inputs/Item.csv")
    adapter["verify"](csv, source)
    items = adapter["parse_items"](csv)
    write("items.json", {str(item["id"]): item for item in items})
    catalogs = []
    for start in range(0, len(items), 100):
        name = f"catalog/{start // 100 + 1}.json"
        write(name, [{k: row[k] for k in ("id", "path", "name", "category")}
                     for row in items[start:start + 100]])
        catalogs.append(name)

    cargo = tomllib.loads(read(args.ranim / "Cargo.toml").decode())
    shadow = tomllib.loads(read(args.ranim / "shadow.toml").decode())
    backend = shadow["backend"]
    endpoint = backend["endpoint"].split("://", 1)[1].rstrip("/")
    prefix = backend.get("prefix", "").strip("/")
    base = f"https://{backend['bucket']}.{endpoint}/" + (prefix + "/" if prefix else "")
    base += shadow["name"] + "/objects/sha256/"
    metadata = cargo.get("package", {}).get("metadata", {}).get("example", {})
    examples = []
    for target in cargo["example"]:
        name = target["name"]
        directory = (args.ranim / target["path"]).parent
        sources = [{"file": file, "text": read(directory / file).decode()}
                   for file in ("scenes.rs", "main.rs")]
        previews, outputs = [], []
        refs = args.ranim / ".shadow/refs/website/static/examples" / name
        mime = {".mp4": "video/mp4", ".webm": "video/webm", ".gif": "image/gif",
                ".png": "image/png", ".jpg": "image/jpeg"}
        for ref in sorted(refs.rglob("*.ref")):
            file = ref.relative_to(refs).as_posix()[:-4]
            ext = Path(file).suffix
            if ext not in mime:
                continue
            oid = tomllib.loads(read(ref).decode())["oid"].removeprefix("sha256:")
            if len(oid) != 64 or any(c not in "0123456789abcdefABCDEF" for c in oid):
                raise ValueError(f"invalid sha256 ref: {ref}")
            url = base + oid[:2] + "/" + oid[2:]
            if ext in (".png", ".jpg"):
                previews.append(url)
            outputs.append({"url": url, "kind": mime[ext]})
        pkg = args.ranim / "website/static/examples" / name / "pkg"
        wasm = (pkg / "ranim_example.js").is_file() and (pkg / "ranim_example_bg.wasm").is_file()
        record = {"name": name, "sources": sources, "preview_imgs": previews,
                  "output_files": outputs, "wasm": wasm,
                  "hidden": metadata.get(name, {}).get("hide", False)}
        write(f"ranim/{name}.json", record)
        examples.append(name)
    # Verify an immutable byte snapshot, without touching publication inputs.
    for name, version in versions.items():
        if digest(Path(name).read_bytes()) != version:
            raise RuntimeError(f"consumer input changed during snapshot: {name}")
    write("manifest.json", {"csv_sha256": digest(csv), "items": len(items),
          "ids": [str(item["id"]) for item in items], "catalogs": catalogs,
          "ranim": examples, "source_versions": versions,
          "scope": "standalone non-production bodies, not consumer migration or full player"})
    print(json.dumps({"fixture": str(out), "items": len(items), "catalogs": len(catalogs),
                      "ranim": len(examples), "source_files": len(versions)}, ensure_ascii=False))


if __name__ == "__main__":
    main()
