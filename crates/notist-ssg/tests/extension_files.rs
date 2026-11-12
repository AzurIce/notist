use notist::{MemoryResources, Vault};
use notist_ssg::{
    Builder, Error, SiteConfig,
    extensions::{SiteContext, SiteExtension, TemplateInput},
};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone)]
struct Files(BTreeMap<std::path::PathBuf, Vec<u8>>);

impl SiteExtension for Files {
    fn name(&self) -> &str {
        "files"
    }

    fn before_template(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut TemplateInput<'_>,
    ) -> Result<(), Error> {
        assert!(input.pages[0].html.contains("Home"));
        input.files.extend(self.0.clone());
        input.data.insert("asset".into(), "assets/raw.bin".into());
        Ok(())
    }
}

fn vault() -> Vault<MemoryResources> {
    let mut resources = MemoryResources::new("/site");
    resources.insert("content/index.not", "= Home");
    resources.insert("static/existing.txt", "physical");
    resources.insert("sass/main.scss", "body { color: red; }");
    resources.insert(
        "templates/page.tera",
        "{{site.data.asset | default(value='none')}}|{{page.html | safe}}",
    );
    Vault::new(resources)
}

fn files(entries: &[(&str, &[u8])]) -> Files {
    Files(
        entries
            .iter()
            .map(|(path, bytes)| ((*path).into(), bytes.to_vec()))
            .collect(),
    )
}

#[test]
fn contributed_files_are_identical_in_build_and_serve_and_not_written_to_inputs() {
    let builder = Builder::default().with_extension(files(&[
        ("assets/raw.bin", &[0, 255, 1]),
        (".well-known/example", b"allowed"),
        ("search-index.json", br#"[{"title":"Home"}]"#),
    ]));
    let mut vault = vault();
    let build = builder.build(&mut vault, &SiteConfig::default()).unwrap();
    let serve = builder.serve(&mut vault, &SiteConfig::default()).unwrap();
    assert_eq!(build.files, serve.files);
    assert_eq!(build.files[Path::new("assets/raw.bin")], [0, 255, 1]);
    assert_eq!(build.files[Path::new(".well-known/example")], b"allowed");
    assert!(
        String::from_utf8_lossy(&build.files[Path::new("index.html")]).contains("assets/raw.bin|")
    );
    let fresh = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    assert!(!fresh.files.contains_key(Path::new("assets/raw.bin")));
    assert!(!fresh.files.contains_key(Path::new("search-index.json")));
}

#[derive(Debug, Clone)]
struct EditFiles;

impl SiteExtension for EditFiles {
    fn name(&self) -> &str {
        "edit-files"
    }

    fn before_template(
        &mut self,
        _: &SiteContext<'_>,
        input: &mut TemplateInput<'_>,
    ) -> Result<(), Error> {
        assert_eq!(input.files[Path::new("replace.txt")], b"before");
        input.files.insert("replace.txt".into(), b"after".to_vec());
        input.files.remove(Path::new("remove.txt"));
        Ok(())
    }
}

#[test]
fn later_extensions_can_edit_only_the_contributed_file_collection() {
    let site = Builder::default()
        .with_extension(files(&[
            ("replace.txt", b"before"),
            ("remove.txt", b"gone"),
        ]))
        .with_extension(EditFiles)
        .build(&mut vault(), &SiteConfig::default())
        .unwrap();
    assert_eq!(site.files[Path::new("replace.txt")], b"after");
    assert!(!site.files.contains_key(Path::new("remove.txt")));
    assert_eq!(site.files[Path::new("existing.txt")], b"physical");
}

#[test]
fn invalid_reserved_and_conflicting_paths_fail_with_extension_context() {
    for path in [
        "",
        "/absolute",
        "../escape",
        "a/../escape",
        "a/./b",
        "a//b",
        "a/",
        "a\\b",
        "a\0b",
        "_notist",
        "_notist/custom.js",
        "index.html",
        "index.html/child",
        "existing.txt",
        "existing.txt/child",
        "main.css",
    ] {
        let error = Builder::default()
            .with_extension(files(&[(path, b"bad")]))
            .build(&mut vault(), &SiteConfig::default())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("site extension `files` (before_template)"),
            "{path:?}: {error}"
        );
        assert!(
            error.contains("file path") || error.contains("collision"),
            "{path:?}: {error}"
        );
    }
    for entries in [
        vec![("a", b"one".as_slice()), ("a/b", b"two".as_slice())],
        vec![
            ("assets", b"one".as_slice()),
            ("assets/raw.bin", b"two".as_slice()),
        ],
    ] {
        let error = Builder::default()
            .with_extension(files(&entries))
            .build(&mut vault(), &SiteConfig::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("collision"), "{error}");
    }
}
