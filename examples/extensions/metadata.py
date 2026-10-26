"""A document extension driven by the notist CLI, using Python's standard library."""
import json
import sys
import tomllib
from pathlib import Path


def text(value):
    return {
        "Call": {
            "callee": {"Ref": {"name": "notist.text", "span": [0, 0]}},
            "args": [{"Named": {"name": "text", "value": {"Literal": [{"Str": value}, [0, 0]]}}}],
            "comptime": [],
            "children": [],
            "body": "None",
            "attrs": [],
            "span": [0, 0],
        }
    }


request = json.load(sys.stdin)
if request["version"] != 1 or request["stage"] != "before_check":
    raise ValueError("unsupported extension request")

package = tomllib.loads(Path("../../Cargo.toml").read_text())["package"]
document = request["document"]
document["forest"].append({
    "Fragment": {
        "children": [text(f"Package: {package['name']} {package['version']}")],
        "span": [0, 0],
    }
})
document["attrs"].append(["package", {"Str": package["name"]}])
json.dump({"version": 1, "document": document}, sys.stdout)
