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
    manifest_path = manifest_path.resolve()
    project = manifest_path.parent
    manifest = tomllib.loads(manifest_path.read_text())
    documents = {
        path: source for path, source in request["documents"].items()
        if not path.startswith("examples/")
    }
    documents["examples/index.md"] = annotation(title="Examples", template="examples") + "# Examples\n"
    metadata = {}
    for example in manifest.get("example", []):
        name = example["name"]
        entry = project / example["path"]
        group = entry.parent.relative_to(project / "examples").parent
        directory = Path("examples") / group
        page = directory / f"{name}.md"
        documents[page.as_posix()] = annotation(
            title=name, template="example", example=name
        ) + f"# {name}\n"
        for parent in directory.parents:
            if parent == Path("."):
                break
            key = (parent / "index.md").as_posix()
            documents.setdefault(key, annotation(title=parent.name, template="examples"))
        if directory != Path("examples"):
            documents.setdefault((directory / "index.md").as_posix(), annotation(title=directory.name, template="examples"))
        sources = [entry.with_name("scenes.rs"), entry]
        code = "\n\n".join(
            f"`{path.name}`\n\n```rust\n{path.read_text().rstrip()}\n```"
            for path in sources if path.is_file()
        )
        documents[page.as_posix()] += "\n" + code + "\n"
        data = metadata.setdefault(name, {})
        data.update(name=name, code=code)
        data["wasm"] = (Path(request["root"]) / "static" / "examples" / name / "pkg" / "ranim_example.js").is_file()
    return documents, metadata


if __name__ == "__main__":
    metadata = {}
    for line in sys.stdin:
        request = json.loads(line)
        response = {"version": 2, "event": request["event"]}
        try:
            if request["version"] != 2:
                raise ValueError("unsupported site extension request")
            if request["event"] == "before_compile":
                response["documents"], metadata = generate(request, Path(sys.argv[1]))
            elif request["event"] == "before_template":
                data = request["data"]
                for name, values in metadata.items():
                    data.setdefault(name, {}).update(values)
                response["data"] = data
            else:
                raise ValueError("unsupported event")
        except Exception as error:
            response["errors"] = [str(error)]
        print(json.dumps(response), flush=True)
