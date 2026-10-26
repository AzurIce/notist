"""Experimental site extension: generate a Cargo example catalog without files.

The extension owns documents under examples/. Existing data entries (including
published Ranim media URLs and render hashes) are retained and source code is
read afresh. It never compiles Rust, renders media or uploads assets.
"""
import json
import sys
import tomllib
from pathlib import Path


def annotation(**values):
    return "@!(" + ", ".join(
        f"{key}: {json.dumps(value, ensure_ascii=False)}"
        for key, value in values.items()
    ) + ")\n\n"


def generate(request, manifest_path):
    if request["version"] != 1 or request["stage"] != "prepare":
        raise ValueError("unsupported site extension request")
    manifest_path = manifest_path.resolve()
    project = manifest_path.parent
    manifest = tomllib.loads(manifest_path.read_text())
    site = request["input"]
    site["documents"] = {
        path: source for path, source in site["documents"].items()
        if not path.startswith("examples/")
    }
    site["documents"]["examples/index.md"] = annotation(title="Examples", template="examples") + "# Examples\n"
    for example in manifest.get("example", []):
        name = example["name"]
        entry = project / example["path"]
        group = entry.parent.relative_to(project / "examples").parent
        directory = Path("examples") / group
        page = directory / f"{name}.md"
        site["documents"][page.as_posix()] = annotation(
            title=name, template="example", example=name
        ) + f"# {name}\n"
        for parent in directory.parents:
            if parent == Path("."):
                break
            key = (parent / "index.md").as_posix()
            site["documents"].setdefault(key, annotation(title=parent.name, template="examples"))
        if directory != Path("examples"):
            site["documents"].setdefault((directory / "index.md").as_posix(), annotation(title=directory.name, template="examples"))
        sources = [entry.with_name("scenes.rs"), entry]
        code = "\n\n".join(
            f"`{path.name}`\n\n```rust\n{path.read_text().rstrip()}\n```"
            for path in sources if path.is_file()
        )
        data = site["data"].setdefault(name, {})
        data.update(name=name, code=code)
        data["wasm"] = (Path(request["root"]) / "static" / "examples" / name / "pkg" / "ranim_example.js").is_file()
    return {"version": 1, "input": site}


if __name__ == "__main__":
    try:
        response = generate(json.load(sys.stdin), Path(sys.argv[1]))
    except Exception as error:
        response = {"version": 1, "errors": [str(error)]}
    json.dump(response, sys.stdout)
