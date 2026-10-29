# Experimental SSG input extensions

From the Notist repository root, run:

```sh
cargo run -p notist-cli -- build -p ssg-extensions
```

Requires Python 3.11+. The command reads `project/Cargo.toml` and its example
sources and generates the catalog, grouped pages, and `site.data` in memory.
Open `dist/ssg-extensions-site/index.html` to inspect the output.

`examples.py` accepts another Cargo manifest as its argument. It follows the
`examples/<group>/<name>/main.rs` layout used by Ranim, also includes adjacent
`scenes.rs`, and keeps existing template data such as published media URLs and
render hashes. The extension owns the `examples/` document namespace. It is a
generation example, not a migration of Ranim's rendering/publishing workflow.

Native extensions implement `notist_ssg::extensions::SiteExtension` and register
through `notist_ssg::Builder::with_extension`. Commands use `[[site.extensions]]`.
This API and protocol are experimental and can change as use cases develop.
