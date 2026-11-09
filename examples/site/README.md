# Notist site example

This package demonstrates templates, static resources, Sass, and site data.
From the Notist repository root, run:

```sh
cargo run -p notist-cli -- build -p notist-site
cargo run -p notist-cli -- serve -p notist-site
```

Build output is `notist-output/notist-site/` at the repository root. Selecting
this member by name does not change the workspace's shared output root.
`serve` prints the local preview URL and keeps generated files in memory.

The Notist workspace is independent of the Cargo workspace. Adding a
`Notist.toml` here does not register or change a Cargo example target.
