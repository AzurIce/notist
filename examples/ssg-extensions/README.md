# Experimental SSG input extensions

From the Notist repository root, run:

```sh
cargo run -p notist-cli -- build -p ssg-extensions
cargo run -p notist-cli -- serve -p ssg-extensions
```

Requires Python 3.11+. The command reads `project/Cargo.toml` and its example
sources and generates the catalog, grouped pages, and `site.data` in memory.
The package is a member of the repository's Notist workspace. Build output is
`notist-output/ssg-extensions/` at the repository root, even when the member
is selected by name. Use `serve` to inspect the site locally; preview
builds stay in memory and do not write that directory.

`cargo run -p notist-cli` selects the Rust CLI crate; `-p ssg-extensions` after `--`
selects the Notist package. To build all workspace sites, run
`cargo run -p notist-cli -- build --workspace`.

`examples.py` accepts another Cargo manifest as its argument. It follows the
`examples/<group>/<name>/main.rs` layout used by Ranim, also includes adjacent
`scenes.rs`, and keeps existing template data such as published media URLs and
render hashes. The extension owns the `examples/` document namespace. It is a
generation example, not a migration of Ranim's rendering/publishing workflow.

Each command process handles `before_compile` and `before_template` JSON Lines
events in one build. It keeps generated metadata in memory between events;
only the second event receives template data. Flush one response per event and
exit on stdin EOF. Each rebuild starts a new process.

Native extensions implement `notist_ssg::extensions::SiteExtension` and `Clone`, and register
through `notist_ssg::Builder::with_extension`. A fresh clone handles both events in each build.
Commands use `[site.extensions.<name>]`;
built-ins use `[site.extensions.builtins.<name>]`. Set `extension_order` on the site
to order dependent extensions explicitly; otherwise configured names are sorted.
This API and protocol are experimental and can change as use cases develop.
