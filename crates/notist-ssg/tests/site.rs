use notist::{MemoryResources, Vault};
use notist_ssg::{Builder, SiteConfig};
use std::path::Path;

fn vault(files: &[(&str, &str)]) -> Vault<MemoryResources> {
    let mut resources = MemoryResources::new("/site");
    for (path, text) in files {
        resources.insert(path, text.as_bytes().to_vec());
    }
    Vault::new(resources)
}
fn html(site: &notist_ssg::BuildOutput, path: &str) -> String {
    String::from_utf8(site.files[Path::new(path)].clone()).unwrap()
}

#[test]
fn maps_sources_without_inventing_directory_pages_and_resolves_links() {
    let mut vault = vault(&[
        (
            "content/index.not",
            "= Home\n\n[Guide](guide/start.md?mode=1#topic)",
        ),
        (
            "content/guide/start.md",
            "# Start\n\n## Topic\n\n[Home](../index.not)\n\n![Picture](a%20b.svg)",
        ),
        ("content/guide/a b.svg", "<svg/>"),
        ("content/guide/module.notc", "private module"),
        ("content/guide/.private", "private"),
        ("content/README.not", "= Readme"),
    ]);
    let site = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    assert_eq!(site.pages.len(), 3);
    assert!(site.warnings.is_empty());
    assert!(html(&site, "index.html").contains("guide/start.html?mode=1#topic"));
    let guide = html(&site, "guide/start.html");
    assert!(guide.contains("href=\"../index.html\""));
    assert!(guide.contains("id=\"topic\""));
    assert!(guide.contains("<img src=\"a%20b.svg\" alt=\"Picture\">"));
    assert!(site.files.contains_key(Path::new("README.html")));
    assert!(!site.files.contains_key(Path::new("guide/index.html")));
    assert!(!site.files.contains_key(Path::new("guide/module.notc")));
    assert!(!site.files.contains_key(Path::new("guide/.private")));
}

#[test]
fn templates_consume_the_directory_tree_data_and_override_the_theme() {
    let mut vault = vault(&[
        (
            "content/index.not",
            "@!(title: \"Home & more\", template: \"home\")\n= Body",
        ),
        ("content/folder/a.not", "= A"),
        ("content/empty/asset.txt", "asset"),
        ("theme/templates/home.hbs", "theme"),
        ("theme/templates/part.hbs", "{{site.data.example.name}}"),
        ("theme/static/style.css", "theme css"),
        ("static/style.css", "site css"),
        ("data/example.toml", "name='Example'\ncode='**strong**'"),
        (
            "templates/home.hbs",
            "{{page.title}}|{{> part}}|{{url 'style.css'}}|{{#each site.directories}}{{path}};{{/each}}|{{{markdown site.data.example.code}}}|{{{page.html}}}",
        ),
    ]);
    let config =
        SiteConfig::parse("[ssg]\nbase_url='https://example.com/project'\ntheme='theme'").unwrap();
    let site = Builder::default().build(&mut vault, &config).unwrap();
    let home = html(&site, "index.html");
    assert!(home.contains("Home &amp; more|Example|https://example.com/project/style.css"));
    assert!(home.contains("folder;"));
    assert!(home.contains("empty;"));
    assert!(home.contains("<strong>strong</strong>"));
    assert!(home.contains("<h1 id=\"body\">Body</h1>"));
    assert_eq!(site.files[Path::new("style.css")], b"site css");
}

#[test]
fn sass_uses_the_supplied_resources_including_partials() {
    let mut vault = vault(&[
        ("content/index.not", "= Home"),
        ("sass/index.scss", "@import 'colors'; body { color: $ink; }"),
        ("sass/_colors.scss", "$ink: #123456;"),
    ]);
    let site = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    assert!(html(&site, "index.css").contains("color: #123456"));
    assert!(!site.files.contains_key(Path::new("_colors.css")));
}

#[test]
fn rejects_all_output_collisions_and_overlapping_sources() {
    for files in [
        vec![("content/a.not", "= A"), ("content/a.md", "# A")],
        vec![("content/a.not", "= A"), ("static/a.html", "collision")],
        vec![
            ("content/a.not", "= A"),
            ("static/a.html/image.png", "collision"),
        ],
        vec![
            ("content/a.not", "= A"),
            ("content/_notist/data.txt", "reserved"),
        ],
    ] {
        let error = Builder::default()
            .build(&mut vault(&files), &SiteConfig::default())
            .unwrap_err();
        assert!(
            error.to_string().contains("collision") || error.to_string().contains("reserved"),
            "{error}"
        );
    }
    let config = SiteConfig {
        output: "content/output".into(),
        ..SiteConfig::default()
    };
    assert!(
        Builder::default()
            .build(&mut vault(&[("content/a.not", "= A")]), &config)
            .unwrap_err()
            .to_string()
            .contains("overlap")
    );
}

#[test]
fn broken_links_warn_but_missing_embeds_and_invalid_templates_fail() {
    let site = Builder::default()
        .build(
            &mut vault(&[("content/index.not", "= Home\n\n[Missing](missing.not)")]),
            &SiteConfig::default(),
        )
        .unwrap();
    assert_eq!(site.warnings.len(), 1);
    assert!(
        Builder::default()
            .build(
                &mut vault(&[("content/index.not", "= Home\n\n![Missing](missing.svg)")]),
                &SiteConfig::default()
            )
            .is_err()
    );
    assert!(
        Builder::default()
            .build(
                &mut vault(&[("content/index.not", "@!(template: \"missing\")\n= Home")]),
                &SiteConfig::default()
            )
            .is_err()
    );
    assert!(
        Builder::default()
            .build(
                &mut vault(&[(
                    "content/index.not",
                    "@(id: \"x\")\n= One\n\n@(id: \"x\")\n== Two"
                )]),
                &SiteConfig::default()
            )
            .is_err()
    );
}

#[test]
fn native_components_publish_their_resources_and_registration_script() {
    let mut vault = vault(&[
        (
            "content/index.not",
            "#import \"../module/lib.notc\" as panel;\n#panel.panel[\nContent\n]",
        ),
        (
            "module/lib.notc",
            "fn panel [children: BlockContent] -> BlockContent;",
        ),
        (
            "module/panel/index.js",
            "export default class extends HTMLElement {}",
        ),
    ]);
    let site = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    let page = html(&site, "index.html");
    assert!(page.contains("notist-protocol=\"2\""));
    assert!(page.contains("src=\"_notist/pages/index.js\""));
    let registrations = html(&site, "_notist/pages/index.js");
    assert!(registrations.contains("customElements.define"));
    assert!(
        site.files
            .keys()
            .any(|p| p.starts_with("_notist/modules") && p.ends_with("panel/index.js"))
    );
}

#[test]
fn evaluated_heading_ids_and_code_highlighting_are_shared_with_template_markdown() {
    let mut vault = vault(&[
        (
            "content/index.not",
            "@(id: \"explicit\")\n= One\n\n== One\n\n== One\n\n```rust\nlet x = 1 < 2;\n```",
        ),
        (
            "templates/page.hbs",
            "{{{page.html}}}{{{markdown site.data.code.source}}}",
        ),
        ("data/code.toml", "source='''```rust\nfn main() {}\n```'''"),
    ]);
    let site = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    let page = html(&site, "index.html");
    assert_eq!(page.matches("id=\"explicit\"").count(), 1);
    assert!(page.contains("id=\"one\""));
    assert!(page.contains("id=\"one-2\""));
    assert_eq!(page.matches("class=\"z-source z-rust\"").count(), 2);
    assert!(page.contains("&lt;"));
}
