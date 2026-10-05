use notist::{MemoryResources, Vault};
use notist_ssg::{Error, Routes, Site, SiteConfig, build};
use std::path::{Path, PathBuf};

fn resources(files: &[(&str, &str)]) -> MemoryResources {
    let mut resources = MemoryResources::new("/vault");
    for (path, source) in files {
        resources.insert(path, source.as_bytes().to_vec());
    }
    resources
}
fn render(files: &[(&str, &str)]) -> Site {
    build(&mut Vault::new(resources(files)), &SiteConfig::default()).unwrap()
}
fn html(site: &Site, path: &str) -> String {
    String::from_utf8(site.files[Path::new(path)].clone()).unwrap()
}

#[test]
fn mixed_frontends_share_directory_routes_anchors_and_assets() {
    let site = render(&[
        (
            "README.not",
            "= Home\n\n[Guide](guide/README.md?mode=1#topic)",
        ),
        (
            "guide/README.md",
            "# Guide\n\n## Topic\n\n[Home](../README.not) ![Picture](../media/a%20b.svg)",
        ),
        ("media/a b.svg", "<svg/>"),
    ]);
    assert_eq!(site.pages.len(), 2);
    assert!(html(&site, "index.html").contains("href=\"guide/?mode=1#topic\""));
    let guide = html(&site, "guide/index.html");
    assert!(guide.contains("href=\"../\""));
    assert!(guide.contains("href=\"../media/a%20b.svg\""));
    assert!(guide.contains("id=\"topic\""));
    assert!(guide.contains("href=\"#topic\""));
    assert_eq!(site.files[Path::new("media/a b.svg")], b"<svg/>");
}

#[test]
fn docs_vault_excludes_repository_content_but_loads_external_packages() {
    let mut files = MemoryResources::new("/repo/docs");
    for (path, source) in [
        ("/repo/README.not", "= Repository"),
        (
            "README.not",
            "= Documentation\n\n#widgets::badge(\"Ready\")",
        ),
        ("guide.md", "# Guide"),
        (
            "Notist.toml",
            "[dependencies]\nwidgets={path='../packages/widgets'}",
        ),
        (
            "/repo/packages/widgets/Notist.toml",
            "[package]\nname='widgets'",
        ),
        (
            "/repo/packages/widgets/lib.notc",
            "fn badge(label: String) -> InlineContent;",
        ),
        (
            "/repo/packages/widgets/components/badge.js",
            "export default class extends HTMLElement {}",
        ),
        ("/repo/packages/widgets/README.not", "= Widgets"),
    ] {
        files.insert(path, source.as_bytes().to_vec());
    }
    let mut vault = Vault::new(files.clone());
    let site = build(&mut vault, &SiteConfig::default()).unwrap();
    assert_eq!(site.pages.len(), 2);
    assert_eq!(site.pages[0].route.url, "/");
    assert_eq!(site.pages[0].title, "Documentation");
    assert!(html(&site, "index.html").contains("<widgets-badge "));
    assert!(
        site.files
            .contains_key(Path::new("_notist/packages/widgets/components/badge.js"))
    );

    files.insert(
        "README.not",
        b"= Documentation\n\n[Widgets](../packages/widgets/README.not)".to_vec(),
    );
    let error = build(&mut Vault::new(files), &SiteConfig::default()).unwrap_err();
    assert!(error.to_string().contains("outside Vault"));
}

#[test]
fn titles_do_not_change_routes_and_groups_do_not_create_pages() {
    let site = render(&[
        ("notes/10.md", "# Ten"),
        ("notes/2.not", "= Two"),
        ("中文 文档.md", "# Title"),
    ]);
    assert!(!site.files.contains_key(Path::new("notes/index.html")));
    assert_eq!(site.pages[0].title, "Two");
    assert_eq!(
        site.pages
            .iter()
            .find(|page| page.title == "Title")
            .unwrap()
            .route
            .url,
        "/%E4%B8%AD%E6%96%87%20%E6%96%87%E6%A1%A3/"
    );
    let page = html(&site, "notes/2/index.html");
    assert!(page.contains("<span>notes</span>"));
    assert!(page.contains("rel=\"next\" href=\"../10/\""));
}

#[test]
fn colliding_readmes_and_file_directory_routes_fail() {
    for sources in [["README.not", "README.md"], ["foo.not", "foo/README.md"]] {
        let error = Routes::new(
            Path::new("/vault"),
            sources.map(|path| PathBuf::from("/vault").join(path)),
        )
        .unwrap_err();
        assert!(error.contains("route collision"));
    }
}

#[test]
fn explicit_section_ids_and_duplicate_slugs_use_one_outline() {
    let site = render(&[(
        "README.not",
        "@(id: \"hello\")\n= Custom\n\n== Hello\n\n== Hello\n\n[Link](#hello-3)",
    )]);
    let page = &site.pages[0];
    assert_eq!(
        page.headings
            .iter()
            .map(|heading| heading.id.as_str())
            .collect::<Vec<_>>(),
        ["hello", "hello-2", "hello-3"]
    );
    let output = html(&site, "index.html");
    assert_eq!(output.matches("id=\"hello\"").count(), 1);
    assert!(output.contains("<section id=\"hello\""));
    assert!(output.contains("href=\"#hello-3\""));
}

#[test]
fn invalid_references_return_source_diagnostics_and_no_site() {
    for (target, message) in [
        ("other.md#absent", "missing anchor"),
        ("missing.png", "missing local resource"),
        ("../secret.md", "outside Vault"),
        ("%2e%2e/secret.svg", "outside Vault"),
    ] {
        let source = format!("# Home\n\n[Target]({target})");
        let mut files = resources(&[("README.md", &source), ("other.md", "# Other")]);
        files.insert("/secret.md", b"# Secret".to_vec());
        let error = build(&mut Vault::new(files), &SiteConfig::default()).unwrap_err();
        let Error::Diagnostics(errors) = error else {
            panic!("expected source diagnostics: {error}");
        };
        assert!(
            errors
                .iter()
                .any(|error| error.diagnostic.message.contains(message)),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .all(|error| error.path == Path::new("/vault/README.md")
                    && usize::from(error.diagnostic.span.end()) <= source.len())
        );
    }
}

#[test]
fn excluded_documents_remain_inside_vault_but_are_unpublished() {
    let config = SiteConfig {
        exclude: vec!["draft/**".into()],
        ..SiteConfig::default()
    };
    let mut vault = Vault::new(resources(&[
        ("README.md", "# Home\n\n[Draft](draft/secret.md)"),
        ("draft/secret.md", "# Secret"),
    ]));
    assert!(
        build(&mut vault, &config)
            .unwrap_err()
            .to_string()
            .contains("not published")
    );
}

#[test]
fn theme_overrides_use_handlebars_partials_and_escape_metadata() {
    let mut vault = Vault::new(resources(&[
        ("README.md", "# \\<Home\\> & title\n\nBody"),
        (
            "theme/index.hbs",
            "{{> title page}}|{{{page.html}}}|{{#each assets.styles}}{{this}}{{/each}}",
        ),
        ("theme/title.hbs", "{{title}}"),
        ("theme/assets/custom.css", "body { color: green; }"),
    ]));
    let config = SiteConfig {
        theme: Some("theme".into()),
        ..SiteConfig::default()
    };
    let site = build(&mut vault, &config).unwrap();
    let output = html(&site, "index.html");
    assert!(output.starts_with("&lt;Home&gt; &amp; title|"), "{output}");
    assert!(output.contains("<p>Body</p>"));
    assert!(
        site.files
            .contains_key(Path::new("_notist/theme/custom.css"))
    );
    assert!(site.files.contains_key(Path::new("_notist/theme/site.css")));
}

#[test]
fn template_errors_fail_build_instead_of_publishing_empty_values() {
    for template in ["{{page.missing}}", "{{#if page.title}}oops"] {
        let mut vault = Vault::new(resources(&[
            ("README.md", "# Home"),
            ("theme/index.hbs", template),
        ]));
        let config = SiteConfig {
            theme: Some("theme".into()),
            ..SiteConfig::default()
        };
        assert!(matches!(
            build(&mut vault, &config),
            Err(Error::Template(_))
        ));
    }
}

#[test]
fn default_transforms_publish_shared_component_resources_per_used_page() {
    let site = render(&[
        ("Notist.toml", "[dependencies]\nmath = {path='../math'}"),
        (
            "/math/Notist.toml",
            "[package]\nname='math'\n[[transforms]]\nkind='replace'\nfrom='notist::math'\nto='math::formula'",
        ),
        (
            "/math/lib.notc",
            "fn formula(text: String) -> InlineContent;",
        ),
        (
            "/math/components/formula/index.js",
            "export default class extends HTMLElement {}",
        ),
        (
            "/math/components/formula/helper.js",
            "export const value=1;",
        ),
        ("README.not", "= Home\n\n$x$"),
        ("other.md", "# Other\n\n$y$"),
        ("plain.md", "# Plain"),
    ]);
    assert!(html(&site, "index.html").contains("<math-formula "));
    assert!(site.files.contains_key(Path::new(
        "_notist/packages/math/components/formula/helper.js"
    )));
    assert!(site.files.contains_key(Path::new("components.js")));
    assert!(site.files.contains_key(Path::new("other/components.js")));
    assert!(!site.files.contains_key(Path::new("plain/components.js")));
    let registration =
        String::from_utf8(site.files[Path::new("other/components.js")].clone()).unwrap();
    assert!(registration.contains("../_notist/packages/math/components/formula/index.js"));
}

#[test]
fn site_configuration_is_separate_from_package_configuration() {
    let config=SiteConfig::parse("[package]\nname='docs'\n[site]\ntitle='Book'\ninclude=['docs/**']\nexclude=['docs/drafts/**']\ntheme='theme'").unwrap();
    assert_eq!(config.title, "Book");
    assert_eq!(config.theme, Some(PathBuf::from("theme")));
    assert!(SiteConfig::parse("[site]\nunknown=true").is_err());
}

#[test]
fn component_sources_cannot_contain_site_output() {
    let mut vault = Vault::new(resources(&[
        ("Notist.toml", "[dependencies]\nwidgets={path='../widgets'}"),
        ("/widgets/Notist.toml", "[package]\nname='widgets'"),
        (
            "/widgets/lib.notc",
            "fn panel()[children: Content] -> Content;",
        ),
        (
            "/widgets/components/panel/index.js",
            "export default class extends HTMLElement {}",
        ),
        ("README.not", "#widgets::panel[Hello]"),
    ]));
    let config = SiteConfig {
        output: "/widgets/components/panel/site".into(),
        ..SiteConfig::default()
    };
    assert!(
        build(&mut vault, &config)
            .unwrap_err()
            .to_string()
            .contains("source and site output overlap")
    );
}

#[test]
fn page_routes_cannot_use_a_published_asset_as_a_directory() {
    let mut vault = Vault::new(resources(&[
        ("README.md", "# Home\n\n[File](foo)"),
        ("foo", "file"),
        ("foo.md", "# Foo"),
    ]));
    assert!(
        build(&mut vault, &SiteConfig::default())
            .unwrap_err()
            .to_string()
            .contains("output collision")
    );
}

#[test]
fn same_named_packages_in_different_page_environments_are_rejected() {
    let mut vault = Vault::new(resources(&[
        (
            "one/Notist.toml",
            "[dependencies]\nwidgets={path='../../first'}",
        ),
        (
            "two/Notist.toml",
            "[dependencies]\nwidgets={path='../../second'}",
        ),
        ("one/README.not", "#widgets::badge(\"One\")"),
        ("two/README.not", "#widgets::badge(\"Two\")"),
        ("/first/Notist.toml", "[package]\nname='widgets'"),
        ("/second/Notist.toml", "[package]\nname='widgets'"),
        (
            "/first/lib.notc",
            "fn badge(label: String) -> InlineContent;",
        ),
        (
            "/second/lib.notc",
            "fn badge(label: String) -> InlineContent;",
        ),
        (
            "/first/components/badge.js",
            "export default class extends HTMLElement {}",
        ),
        (
            "/second/components/badge.js",
            "export default class extends HTMLElement {}",
        ),
    ]));
    assert!(
        build(&mut vault, &SiteConfig::default())
            .unwrap_err()
            .to_string()
            .contains("conflicting resource roots")
    );
}
