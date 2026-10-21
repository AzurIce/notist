use notist::{MemoryResources, Vault};
use notist_html::components::{component_tag, module_key};
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

// Inspect opening tags without depending on attribute order or complete HTML snapshots.
fn tag_with<'a>(output: &'a str, name: &str, attribute: &str) -> Option<&'a str> {
    output.split('<').find_map(|part| {
        let tag = part.split('>').next()?;
        (tag.starts_with(&format!("{name} ")) && tag.contains(attribute)).then_some(tag)
    })
}

#[test]
fn default_theme_renders_semantic_shell_for_both_frontends() {
    for (source, content) in [
        (
            "README.not",
            "= Native title\n\n== Topic\n\nNative body\n\n```rust\nlet value = 1;\n```",
        ),
        (
            "README.md",
            "# Markdown title\n\n## Topic\n\nMarkdown body\n\n```rust\nlet value = 1;\n```",
        ),
    ] {
        let site = render(&[(source, content), ("next.md", "# Next")]);
        let output = html(&site, "index.html");
        assert!(tag_with(&output, "header", "class=\"site-header\"").is_some());
        assert!(tag_with(&output, "main", "id=\"page-content\"").is_some());
        let navigation = tag_with(&output, "nav", "id=\"site-nav\"").unwrap();
        assert!(navigation.contains("aria-label=\"Contents\""));
        assert!(tag_with(&output, "aside", "class=\"page-outline\"").is_some());
        assert!(tag_with(&output, "nav", "id=\"page-outline\"").is_some());
        assert!(tag_with(&output, "nav", "class=\"breadcrumbs\"").is_some());
        assert!(tag_with(&output, "footer", "class=\"site-footer\"").is_some());
        let toggle = tag_with(&output, "button", "id=\"nav-toggle\"").unwrap();
        assert!(toggle.contains("aria-controls=\"site-nav\""));
        assert!(toggle.contains("aria-expanded=\"false\""));
        assert!(tag_with(&output, "button", "id=\"nav-close\"").is_some());
        let theme_toggle = tag_with(&output, "button", "id=\"theme-toggle\"").unwrap();
        assert!(theme_toggle.contains("aria-label=\""));
        assert!(output.contains(">Contents</button>"));
        assert!(output.contains(">Close</button>"));
        assert!(output.contains("class=\"theme-label\">Theme</span>"));
        assert!(!output.contains("<svg"));
        assert!(!output.contains("site-mark"));
        assert!(!output.contains(['←', '→', '↑']));
        let backdrop = output
            .split('<')
            .find(|tag| tag.contains("id=\"nav-backdrop\""))
            .unwrap();
        assert!(backdrop.split('>').next().unwrap().contains("hidden"));
        let progress = output
            .split('<')
            .find(|tag| tag.contains("id=\"reading-progress\""))
            .unwrap();
        assert!(
            progress
                .split('>')
                .next()
                .unwrap()
                .contains("aria-hidden=\"true\"")
        );
        assert!(
            tag_with(&output, "a", "rel=\"next\"")
                .unwrap()
                .contains("href=\"next/\"")
        );
        assert!(output.contains("href=\"#topic\""));
        let article = output
            .split("<article")
            .nth(1)
            .unwrap()
            .split("</article>")
            .next()
            .unwrap();
        assert_eq!(article.matches("<h1").count(), 1, "{article}");
        assert!(article.contains("body</p>"));
        assert!(article.contains("<pre>"));
        // Copy controls are a browser enhancement, not part of page.html.
        assert!(!article.contains("<button"));
    }
}

#[test]
fn default_theme_escapes_titles_and_encodes_explicit_outline_ids() {
    let site = render(&[
        (
            "README.md",
            "# \\<Home\\> & title\n\n[Native](%E4%B8%AD%E6%96%87%20%E6%96%87%E6%A1%A3.not)",
        ),
        (
            "中文 文档.not",
            "@(id: \"中文 &?#/\")\n= Native & title\n\nNative body",
        ),
    ]);
    let home = html(&site, "index.html");
    assert!(home.contains("&lt;Home&gt; &amp; title"));
    assert!(!home.contains("<Home>"));
    assert!(home.contains("href=\"%E4%B8%AD%E6%96%87%20%E6%96%87%E6%A1%A3/\""));
    let native = html(&site, "中文 文档/index.html");
    assert_eq!(
        site.pages
            .iter()
            .find(|page| page.title == "Native & title")
            .unwrap()
            .headings[0]
            .id,
        "中文 &?#/"
    );
    assert!(native.contains("id=\"中文 &amp;?#/\""));
    let outline = native
        .split("id=\"page-outline\"")
        .nth(1)
        .unwrap()
        .split("</nav>")
        .next()
        .unwrap();
    assert!(
        outline.contains("href=\"#%E4%B8%AD%E6%96%87%20&amp;%3F%23%2F\""),
        "{outline}"
    );
    assert!(outline.contains("Native &amp; title"));
    assert!(!native.contains("Native & title"));
    assert!(
        tag_with(&native, "a", "rel=\"prev\"")
            .unwrap()
            .contains("href=\"../\"")
    );
}

#[test]
fn default_theme_omits_outline_and_adjacency_for_headingless_single_pages() {
    for source in ["README.not", "README.md"] {
        let site = render(&[(source, "Only body text.")]);
        assert!(site.pages[0].headings.is_empty());
        let output = html(&site, "index.html");
        assert!(output.contains("<p>Only body text.</p>"));
        assert!(tag_with(&output, "aside", "class=\"page-outline\"").is_none());
        assert!(tag_with(&output, "nav", "id=\"page-outline\"").is_none());
        assert!(tag_with(&output, "nav", "class=\"adjacent\"").is_none());
        assert!(tag_with(&output, "a", "rel=\"prev\"").is_none());
        assert!(tag_with(&output, "a", "rel=\"next\"").is_none());
        assert!(
            !output.contains("<h1"),
            "the fallback title must not become a body heading"
        );
        assert!(!output.contains("src=\"./components.js\""));
    }
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
fn docs_vault_excludes_repository_content_but_loads_external_modules() {
    let mut files = MemoryResources::new("/repo/docs");
    for (path, source) in [
        ("/repo/README.not", "= Repository"),
        (
            "README.not",
            "#import \"../modules/widgets/lib.notc\" as widgets;\n= Documentation\n\n#widgets.badge(\"Ready\")",
        ),
        ("guide.md", "# Guide"),
        (
            "lib.notc",
            "import \"../modules/widgets/lib.notc\" as widgets;",
        ),
        ("/repo/modules/widgets/Notist.toml", ""),
        (
            "/repo/modules/widgets/lib.notc",
            "fn badge(label: String) -> InlineContent;",
        ),
        (
            "/repo/modules/widgets/badge.js",
            "export default class extends HTMLElement {}",
        ),
        ("/repo/modules/widgets/README.not", "= Widgets"),
    ] {
        files.insert(path, source.as_bytes().to_vec());
    }
    let mut vault = Vault::new(files.clone());
    let site = build(&mut vault, &SiteConfig::default()).unwrap();
    assert_eq!(site.pages.len(), 2);
    assert_eq!(site.pages[0].route.url, "/");
    assert_eq!(site.pages[0].title, "Documentation");
    assert!(html(&site, "index.html").contains(&format!(
        "<{} ",
        component_tag(&notist::FunctionId::new(
            "/repo/modules/widgets/lib.notc",
            "badge"
        ))
    )));
    assert!(site.files.contains_key(Path::new(&format!(
        "_notist/modules/{}/badge.js",
        module_key("/repo/modules/widgets/lib.notc")
    ))));

    files.insert(
        "README.not",
        b"= Documentation\n\n[Widgets](../modules/widgets/README.not)".to_vec(),
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
    let navigation = page
        .split("id=\"site-nav\"")
        .nth(1)
        .unwrap()
        .split("</nav>")
        .next()
        .unwrap();
    assert!(navigation.contains(">notes</"), "{navigation}");
    assert!(
        !navigation.contains("href=\"../\""),
        "a directory group must not link to a synthetic page"
    );
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
    assert!(
        site.files
            .contains_key(Path::new("_notist/theme/status.css"))
    );
    let site_css = output.find("_notist/theme/site.css").unwrap();
    let status_css = output.find("_notist/theme/status.css").unwrap();
    let custom_css = output.find("_notist/theme/custom.css").unwrap();
    assert!(site_css < status_css && status_css < custom_css);
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
fn explicit_transforms_publish_shared_component_resources_per_used_page() {
    let site = render(&[
        ("lib.notc", "import \"../math/lib.notc\" as math;"),
        (
            "Notist.toml",
            "[[transforms]]\nkind='replace'\nimports={math='../math/lib.notc'}\nfrom='notist.math'\nto='math.formula'",
        ),
        (
            "/math/lib.notc",
            "fn formula<C: Type = InlineContent>(text: String) -> C;",
        ),
        (
            "/math/formula/index.js",
            "export default class extends HTMLElement {}",
        ),
        ("/math/formula/helper.js", "export const value=1;"),
        (
            "README.not",
            "#import \"../math/lib.notc\" as math;\n= Home\n\n$x$",
        ),
        (
            "other.md",
            "#import \"../math/lib.notc\" as math;\n# Other\n\n$y$",
        ),
        (
            "guide/deep/README.not",
            "#import \"../../../math/lib.notc\" as math;\n= Nested\n\n$z$",
        ),
        ("plain.md", "#import \"../math/lib.notc\" as math;\n# Plain"),
    ]);
    assert!(html(&site, "index.html").contains(&format!(
        "<{} ",
        component_tag(&notist::FunctionId::new("/math/lib.notc", "formula"))
    )));
    let key = module_key("/math/lib.notc");
    assert!(site.files.contains_key(Path::new(&format!(
        "_notist/modules/{key}/formula/helper.js"
    ))));
    assert!(site.files.contains_key(Path::new("components.js")));
    assert!(site.files.contains_key(Path::new("other/components.js")));
    assert!(!site.files.contains_key(Path::new("plain/components.js")));
    let registration =
        String::from_utf8(site.files[Path::new("other/components.js")].clone()).unwrap();
    assert!(registration.contains(&format!("../_notist/modules/{key}/formula/index.js")));
    let nested = html(&site, "guide/deep/index.html");
    assert!(nested.contains("href=\"../../_notist/theme/site.css\""));
    assert!(nested.contains("src=\"../../_notist/theme/site.js\""));
    assert!(nested.contains("src=\"./components.js\""));
    let nested_registration = html(&site, "guide/deep/components.js");
    assert!(nested_registration.contains(&format!("../../_notist/modules/{key}/formula/index.js")));
}

#[test]
fn site_configuration_is_separate_from_context_configuration() {
    let config = SiteConfig::parse(
        "[site]\ntitle='Book'\ninclude=['docs/**']\nexclude=['docs/drafts/**']\ntheme='theme'",
    )
    .unwrap();
    assert_eq!(config.title, "Book");
    assert_eq!(config.theme, Some(PathBuf::from("theme")));
    assert!(SiteConfig::parse("[site]\nunknown=true").is_err());
}

#[test]
fn component_sources_cannot_contain_site_output() {
    let mut vault = Vault::new(resources(&[
        ("lib.notc", "import \"../widgets/lib.notc\" as widgets;"),
        ("/widgets/Notist.toml", ""),
        (
            "/widgets/lib.notc",
            "fn panel<C: Type>()[children: C] -> BlockContent;",
        ),
        (
            "/widgets/panel/index.js",
            "export default class extends HTMLElement {}",
        ),
        (
            "README.not",
            "#import \"../widgets/lib.notc\" as widgets;\n#widgets.panel[Hello]",
        ),
    ]));
    let config = SiteConfig {
        output: "/widgets/panel/site".into(),
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
fn same_named_modules_in_different_page_contexts_publish_distinct_resources() {
    let mut vault = Vault::new(resources(&[
        (
            "one/lib.notc",
            "import \"../../first/lib.notc\" as widgets;",
        ),
        (
            "two/lib.notc",
            "import \"../../second/lib.notc\" as widgets;",
        ),
        (
            "one/README.not",
            "#import \"../../first/lib.notc\" as widgets;\n#widgets.badge(\"One\")",
        ),
        (
            "two/README.not",
            "#import \"../../second/lib.notc\" as widgets;\n#widgets.badge(\"Two\")",
        ),
        ("/first/Notist.toml", ""),
        ("/second/Notist.toml", ""),
        (
            "/first/lib.notc",
            "fn badge(label: String) -> InlineContent;",
        ),
        (
            "/second/lib.notc",
            "fn badge(label: String) -> InlineContent;",
        ),
        (
            "/first/badge.js",
            "export default class extends HTMLElement {}",
        ),
        (
            "/second/badge.js",
            "export default class extends HTMLElement {}",
        ),
    ]));
    let site = build(&mut vault, &SiteConfig::default()).unwrap();
    for module in ["/first/lib.notc", "/second/lib.notc"] {
        assert!(site.files.contains_key(Path::new(&format!(
            "_notist/modules/{}/badge.js",
            module_key(module)
        ))));
    }
    let first = component_tag(&notist::FunctionId::new("/first/lib.notc", "badge"));
    let second = component_tag(&notist::FunctionId::new("/second/lib.notc", "badge"));
    assert_ne!(first, second);
    assert!(html(&site, "one/index.html").contains(&first));
    assert!(html(&site, "two/index.html").contains(&second));
}

#[test]
fn preview_recovers_code_errors_while_build_remains_strict() {
    let files = [
        ("Notist.toml", ""),
        (
            "lib.notc",
            "fn bad() -> InlineContent { compile_error(\"broken\") } fn good() -> InlineContent { [works] }",
        ),
        (
            "README.not",
            "#import \"lib.notc\" as demo;\n#demo.bad() #demo.good()",
        ),
    ];
    assert!(build(&mut Vault::new(resources(&files)), &SiteConfig::default()).is_err());
    let site =
        notist_ssg::preview(&mut Vault::new(resources(&files)), &SiteConfig::default()).unwrap();
    let html = html(&site, "index.html");
    assert!(html.contains("broken"));
    assert!(html.contains("works"));
    assert!(
        site.diagnostics
            .iter()
            .any(|d| d.path == Path::new("/vault/lib.notc"))
    );
    assert!(
        site.diagnostics
            .iter()
            .any(|d| d.source.contains("compile_error"))
    );
}

#[test]
fn preview_does_not_reinterpret_error_nodes_as_links_embeds_or_headings() {
    let files = [
        ("Notist.toml", ""),
        (
            "lib.notc",
            r#"
            fn bad_link() -> InlineContent { [#link(target: "missing.png")[ #callout[ x ] ]] }
            fn bad_embed() -> InlineContent { [#embed(target: "missing.svg", description: "image", title: 1)] }
            fn bad_heading() -> BlockContent { [ #heading<0>[Bad title] ] }
            fn good() -> InlineContent { [survives] }
        "#,
        ),
        (
            "README.not",
            "#import \"lib.notc\" as demo;\n#demo.bad_link() #demo.bad_embed()\n\n#demo.bad_heading()\n\n#demo.good()",
        ),
    ];
    assert!(build(&mut Vault::new(resources(&files)), &SiteConfig::default()).is_err());
    let site =
        notist_ssg::preview(&mut Vault::new(resources(&files)), &SiteConfig::default()).unwrap();
    let output = html(&site, "index.html");
    assert!(output.contains("survives"));
    assert!(output.contains("role=\"alert\""));
    assert!(!site.diagnostics.is_empty());
    assert!(
        site.diagnostics
            .iter()
            .all(|d| d.diagnostic.phase == notist::Phase::Type),
        "{:?}",
        site.diagnostics
    );
    assert!(site.pages[0].headings.is_empty());
    assert_eq!(site.pages[0].title, SiteConfig::default().title);
}

#[test]
fn generated_reference_diagnostics_point_into_the_code_definition() {
    for (target, message) in [
        ("missing.png", "missing local resource"),
        ("other.not#absent", "missing anchor"),
        ("../private.png", "outside Vault"),
    ] {
        let module = format!(
            "{}fn view() -> InlineContent {{ [#link(target: \"{target}\")[click]] }}",
            "// retained source\n".repeat(20)
        );
        let document = "#import \"lib.notc\" as demo;\n#demo.view()";
        let files = [
            ("Notist.toml", ""),
            ("lib.notc", module.as_str()),
            ("README.not", document),
            ("other.not", "= Other"),
        ];
        let error = build(&mut Vault::new(resources(&files)), &SiteConfig::default()).unwrap_err();
        let Error::Diagnostics(errors) = error else {
            panic!("expected source diagnostics: {error}")
        };
        let diagnostic = errors
            .iter()
            .find(|d| d.diagnostic.message.contains(message))
            .unwrap();
        assert_eq!(diagnostic.path, Path::new("/vault/lib.notc"));
        assert_eq!(diagnostic.source, module);
        assert_eq!(
            diagnostic.diagnostic.source.as_deref(),
            Some("/vault/lib.notc")
        );
        let span = diagnostic.diagnostic.span;
        assert!(usize::from(span.start()) > document.len());
        assert!(usize::from(span.end()) <= module.len());
        assert!(module[usize::from(span.start())..usize::from(span.end())].starts_with("#link("));
    }
}

#[test]
fn preview_recovers_function_body_syntax_errors_by_boundary() {
    let files = [
        ("Notist.toml", ""),
        (
            "lib.notc",
            "fn bad() -> InlineContent { 1 + } fn good() -> InlineContent { [works] }",
        ),
        (
            "README.not",
            "#import \"lib.notc\" as demo;\n#demo.bad() #demo.good()",
        ),
    ];
    assert!(build(&mut Vault::new(resources(&files)), &SiteConfig::default()).is_err());
    let site =
        notist_ssg::preview(&mut Vault::new(resources(&files)), &SiteConfig::default()).unwrap();
    let output = html(&site, "index.html");
    assert!(output.contains("works") && output.contains("role=\"alert\""));
    assert!(
        site.diagnostics
            .iter()
            .any(|d| d.diagnostic.phase == notist::Phase::Syntax && d.diagnostic.recovered)
    );
    let global = [("README.not", "@!(id: missing)\nbody")];
    assert!(
        notist_ssg::preview(&mut Vault::new(resources(&global)), &SiteConfig::default()).is_err()
    );
}
