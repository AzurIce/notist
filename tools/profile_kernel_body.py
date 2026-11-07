#!/usr/bin/env python3
"""Supplementary low-level HTML/link/destruction attribution in a frozen copy.

Unlike the detail probe, does not instrument millions of evaluator calls. The
root wrapper measures destruction at its natural field-drop position and does
not release page trees earlier. All produced files must remain byte-identical.
"""
import argparse
import difflib
import json
import os
from pathlib import Path
import shutil
import subprocess

from profile_kernel_site import ENV, guard, inventory, process, REPO
from profile_xiv_history import once, save
from profile_icexiv_refresh import digest
from xiv_history_probes import enter_function, measure_rhs


def instrument(source):
    path = source / "crates/notist-ssg/src/build.rs"
    text = path.read_text()
    text = once(text, "        let mut resolved = BTreeMap::new();",
                '        let links_profile = notist::xiv_profile::span("body.links");\n        let mut resolved = BTreeMap::new();')
    text = once(text, "        let (registry, environment) = ",
                "        drop(links_profile);\n        let (registry, environment) = ")
    for binding, end, label in [
        ("let (registry, environment) = ", ";\n        let rendered", "body.registry"),
        ("let rendered = ", ";\n        errors.extend(rendered", "body.html"),
        ("let registrations = ", ";\n        if !registrations", "body.components"),
    ]:
        head, body = text.split("pub(crate) fn build", 1)
        text = head + "pub(crate) fn build" + measure_rhs(body, binding, end, label)
    text = once(text, "    let by_source = DocumentIndex::new(&documents);",
                '    let by_source = notist::xiv_profile::measure("body.index", || DocumentIndex::new(&documents));')
    path.write_text(text)
    path = source / "crates/notist-ssg/src/document.rs"
    text = enter_function(path.read_text(), "new", 'let _profile = notist::xiv_profile::span("document.prepare");')
    text = once(text, "    pub root: Item,", "    pub root: ProfiledRoot,")
    text = once(text, "            root,\n            source,", "            root: ProfiledRoot(Some(root)),\n            source,")
    text += '''
pub(crate) struct ProfiledRoot(Option<Item>);
impl std::ops::Deref for ProfiledRoot {
    type Target = Item;
    fn deref(&self) -> &Item { self.0.as_ref().unwrap() }
}
impl std::ops::DerefMut for ProfiledRoot {
    fn deref_mut(&mut self) -> &mut Item { self.0.as_mut().unwrap() }
}
impl Drop for ProfiledRoot {
    fn drop(&mut self) {
        if let Some(root) = self.0.take() {
            notist::xiv_profile::measure("body.drop_tree", || drop(root));
        }
    }
}
'''
    path.write_text(text)


def run(args):
    manifest = guard(args)
    source = args.out / "sources/body"
    shutil.copytree(args.out / "sources/coarse", source)
    instrument(source)
    patch = []
    for name in ["crates/notist-ssg/src/build.rs", "crates/notist-ssg/src/document.rs"]:
        before = (args.out / "sources/coarse" / name).read_text()
        patch.extend(difflib.unified_diff(before.splitlines(True), (source / name).read_text().splitlines(True),
                     fromfile="a/"+name, tofile="b/"+name))
    (args.out / "body.patch").write_text("".join(patch))
    shutil.copy2(Path(__file__), args.out / "body-reproducer.py")
    manifest["sources"]["body"] = inventory(source)
    env = dict(os.environ, **ENV, CARGO_TARGET_DIR=str(args.out / "target"))
    with (args.out / "logs/build-body.log").open("wb") as log:
        metadata = json.loads(subprocess.check_output(["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version=1"], cwd=source, env=env))
        packages = [p["name"] for p in metadata["packages"] if p["id"] in metadata["workspace_members"]]
        subprocess.run(["cargo", "clean", "--release", *[v for p in packages for v in ["-p", p]]], cwd=source, env=env,
                       stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(["cargo", "build", "--release", "--locked", "--offline", "-p", "notist-cli", "--bin", "notist", "--features", "profiling"],
                       cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
    shutil.copy2(args.out / "target/release/notist", args.out / "bin/body")
    manifest["binaries"]["body"] = digest(args.out / "bin/body")
    save(args.out / "manifest.json", manifest)
    rows = [process(args, "body", f"body-{i}") for i in range(2)]
    save(args.out / "body-samples.json", rows)
    guard(args)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    args.out = args.out.resolve()
    run(args)
