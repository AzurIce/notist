use notist::{MemoryResources, Vault};
use notist_ssg::{
    Builder, Error, SiteConfig,
    extensions::{SiteContext, SiteExtension, SiteInput},
};
use std::path::Path;

fn vault(files: &[(&str, &str)]) -> Vault<MemoryResources> {
    let mut resources = MemoryResources::new("/site");
    for (path, text) in files {
        resources.insert(path, *text);
    }
    Vault::new(resources)
}

fn config() -> SiteConfig {
    toml::from_str("theme='builtins.book'\n[extensions.builtins.book]").unwrap()
}

fn html(output: &notist_ssg::BuildOutput, path: &str) -> String {
    String::from_utf8(output.files[Path::new(path)].clone()).unwrap()
}

#[test]
fn summary_is_navigation_data_not_a_theme_or_a_publication_filter() {
    let mut vault = vault(&[
        ("content/index.not", "= Home"),
        ("content/a.md", "# Body title"),
        ("content/unlisted.not", "= Still published"),
        (
            "content/SUMMARY.md",
            "# Summary\n\n- [Home](index.not)\n  - [](a.md)",
        ),
    ]);
    let output = Builder::default().build(&mut vault, &config()).unwrap();
    assert_eq!(output.pages.len(), 3);
    assert!(!output.files.contains_key(Path::new("SUMMARY.html")));
    assert!(output.files.contains_key(Path::new("unlisted.html")));
    assert_eq!(
        output.data["book"]["navigation"][0]["children"][0]["title"],
        "Body title"
    );
    assert_eq!(
        output.data["book"]["pages"]["index.not"]["next"]["url"],
        "a.html"
    );
    assert!(html(&output, "a.html").contains("aria-current=\"page\""));
    assert!(output.data["book"]["pages"]["unlisted.not"]["next"].is_null());
    // The shared tree is not duplicated into each page's metadata.
    assert!(
        output.data["book"]["pages"]["a.md"]
            .get("navigation")
            .is_none()
    );
}

#[test]
fn extension_and_theme_are_independent_and_templates_override_by_name() {
    let mut config = config();
    config.theme = None;
    let mut input = vault(&[
        ("content/index.md", "# Home"),
        (
            "templates/page.tera",
            "{{site.data.book.navigation[0].title}}|{{page.html | safe}}",
        ),
    ]);
    let output = Builder::default().build(&mut input, &config).unwrap();
    assert!(html(&output, "index.html").starts_with("Home|"));
    assert!(!output.files.contains_key(Path::new("book.css")));
    config.theme = Some("builtins.book".into());
    let output = Builder::default().build(&mut input, &config).unwrap();
    assert!(html(&output, "index.html").starts_with("Home|"));
    assert!(output.files.contains_key(Path::new("book.css")));
}

#[test]
fn custom_summary_and_static_overrides_use_the_site_pipeline() {
    let mut config = config();
    config.extensions.builtins.book.as_mut().unwrap().summary = Some("outline.not".into());
    config.directory_urls = true;
    config.copy_content_assets = false;
    let output = Builder::default()
        .build(
            &mut vault(&[
                ("content/README.not", "= Home\n\n![Image](image.svg)"),
                ("content/outline.not", "- [Home](README.not)"),
                ("content/image.svg", "image"),
                ("content/private.json", "{}"),
                ("static/book.css", "custom theme css"),
            ]),
            &config,
        )
        .unwrap();
    assert_eq!(html(&output, "book.css"), "custom theme css");
    assert!(output.files.contains_key(Path::new("image.svg")));
    assert!(!output.files.contains_key(Path::new("private.json")));
    assert!(!output.files.contains_key(Path::new("outline/index.html")));
}

#[test]
fn strict_named_configuration_and_complete_explicit_order() {
    for text in [
        "[extensions.builtins.typo]",
        "[extensions.builtins.book]\nsummray='SUMMARY.not'",
        "[extensions.generate]\ncommand=['sh']\nkind='command'",
        "[extensions.generate]\ncommand=[]",
        "extension_order=['builtins.book','builtins.book']\n[extensions.builtins.book]",
        "extension_order=[]\n[extensions.'builtins.book']\ncommand=['sh']",
        "extension_order=['missing']\n[extensions.builtins.book]",
        "[[extensions]]\nkind='command'\ncommand=['sh']",
    ] {
        assert!(
            toml::from_str::<SiteConfig>(text)
                .map_err(|e| e.to_string())
                .and_then(|c| c.validate().map_err(|e| e.to_string()))
                .is_err(),
            "{text}"
        );
    }
    let config: SiteConfig = toml::from_str("extension_order=['generate','builtins.book']\n[extensions.generate]\ncommand=['sh']\n[extensions.builtins.book]").unwrap();
    config.validate().unwrap();
}

#[derive(Debug)]
struct Generate;
impl SiteExtension for Generate {
    fn name(&self) -> &str {
        "generate"
    }
    fn prepare(&self, _: &SiteContext<'_>, input: &mut SiteInput) -> Result<(), Error> {
        input
            .documents
            .insert("SUMMARY.not".into(), "- [Generated](deep/page.not)".into());
        input
            .documents
            .insert("deep/page.not".into(), "= Generated body".into());
        Ok(())
    }
}

#[test]
fn native_generation_can_supply_summary_and_all_pages_without_disk_sources() {
    let output = Builder::default()
        .with_extension(Generate)
        .build(&mut vault(&[]), &config())
        .unwrap();
    assert_eq!(output.pages.len(), 1);
    assert_eq!(output.data["book"]["navigation"][0]["title"], "Generated");
    assert!(html(&output, "deep/page.html").contains("Generated body"));
}

#[test]
fn preview_recovers_content_errors_but_not_summary_errors() {
    let mut input = vault(&[("content/index.not", "= Home\n\n#missing()")]);
    assert!(Builder::default().build(&mut input, &config()).is_err());
    let preview = Builder::default().serve(&mut input, &config()).unwrap();
    assert!(!preview.diagnostics.is_empty());
    let result = Builder::default().serve(
        &mut vault(&[
            ("content/index.not", "= Home"),
            ("content/SUMMARY.not", "- [Missing](missing.not)"),
        ]),
        &config(),
    );
    assert!(result.is_err());
}

#[test]
fn an_output_inside_content_is_never_rediscovered() {
    let mut config = config();
    config.output = "content/public".into();
    let output = Builder::default()
        .build(
            &mut vault(&[
                ("content/index.md", "# Home"),
                ("content/public/stale.md", "#missing()"),
            ]),
            &config,
        )
        .unwrap();
    assert_eq!(output.pages.len(), 1);
}

#[derive(Debug)]
struct Remove;
impl SiteExtension for Remove {
    fn name(&self) -> &str {
        "remove"
    }
    fn prepare(&self, _: &SiteContext<'_>, input: &mut SiteInput) -> Result<(), Error> {
        input.documents.remove(Path::new("a.not"));
        Ok(())
    }
}

#[test]
fn later_extensions_cannot_leave_dangling_navigation_or_overwrite_reserved_data() {
    let mut config = config();
    config.extensions.builtins.book = None;
    let error = Builder::default()
        .with_extension(notist_ssg::builtins::book::Book::default())
        .with_extension(Remove)
        .build(
            &mut vault(&[
                ("content/index.not", "= Home"),
                ("content/a.not", "= A"),
                ("content/SUMMARY.not", "- [Home](index.not)\n- [A](a.not)"),
            ]),
            &config,
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("(enrich): book navigation references removed page")
    );
    let error = Builder::default()
        .build(
            &mut vault(&[("content/index.not", "= Home"), ("data/book.json", "{}")]),
            &self::config(),
        )
        .unwrap_err();
    assert!(error.to_string().contains("site.data.book is reserved"));
}

#[test]
fn directory_urls_keep_source_directory_listings_and_root_relative_links() {
    let mut config = config();
    config.directory_urls = true;
    let output = Builder::default().build(&mut vault(&[
        ("content/README.md", "# Home\n\n[Child](/guide/child.not)"),
        ("content/guide/child.not", "= Child\n\n[Home](/README.md)"),
        ("templates/page.tera", "{{directory.path}}|{% if directory.index %}{{directory.index.title}}{% endif %}|{{page.html | safe}}"),
    ]), &config).unwrap();
    assert!(html(&output, "index.html").starts_with("|Home|"));
    assert!(html(&output, "index.html").contains("href=\"guide/child/\""));
    assert!(html(&output, "guide/child/index.html").starts_with("guide||"));
    assert!(html(&output, "guide/child/index.html").contains("href=\"../../\""));
}
