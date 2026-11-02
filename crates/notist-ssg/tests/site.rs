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

#[test]
fn structured_records_are_build_time_bodies_not_published_data_and_keep_error_sources() {
    let page = "#import \"../views/lib.notc\" as view;\n#view.show()";
    let module = "fn show(source:Path=p\"../private/items.json\")->BlockContent{let root=data_expect<Data>(read_json(source));let name=data_expect<String>(data_require<String>(root,\"name\"));[ #text(text:name) ]}";
    let mut host = vault(&[
        ("content/index.not", page),
        ("content/two.not", page),
        ("views/lib.notc", module),
        (
            "private/items.json",
            r#"{"name":"static <&>","private":"SECRET"}"#,
        ),
    ]);
    let site = Builder::default()
        .build(&mut host, &SiteConfig::default())
        .unwrap();
    for path in ["index.html", "two.html"] {
        let output = html(&site, path);
        assert!(output.contains("static &lt;&amp;&gt;"));
        assert!(!output.contains("SECRET"));
    }
    assert!(!site.files.keys().any(|p| p.ends_with("items.json")));
    assert_eq!(host.input_session().data_cache_stats().parses, 1);
    let data = r#"{"name":false}"#;
    let mut host = vault(&[
        ("content/index.not", page),
        ("views/lib.notc", module),
        ("private/items.json", data),
    ]);
    let Err(notist_ssg::Error::Diagnostics(errors)) =
        Builder::default().build(&mut host, &SiteConfig::default())
    else {
        panic!("expected data diagnostic")
    };
    assert!(
        errors
            .iter()
            .any(|e| e.path == Path::new("/site/private/items.json")
                && e.source == data
                && e.diagnostic.message.contains("/name")),
        "{errors:?}"
    );
}
fn html(site: &notist_ssg::BuildOutput, path: &str) -> String {
    String::from_utf8(site.files[Path::new(path)].clone()).unwrap()
}

#[test]
fn embeds_use_content_roots_bound_module_defaults_and_independent_suffixes() {
    let mut vault = vault(&[
        (
            "content/pages/start.not",
            r#"= Start

#import "../views/lib.notc" as view;
#view.defaulted()

#view.picture(p"../assets/a?b#c%20.svg", fragment: "one") #embed(p"/assets/a?b#c%20.svg", query: "v=2", fragment: "two")

![Legacy](/assets/a%3Fb%23c%2520.svg#three) #embed("https://cdn.test/a.svg#remote")

#embed(p"../other.not", fragment: "part%one")
"#,
        ),
        (
            "content/views/lib.notc",
            r#"fn picture(target: Path, fragment?: String) -> InlineContent { [#embed(target, fragment: fragment)] }
fn defaulted(target: Path = p"./default.svg") -> InlineContent { [#self.picture(target)] }"#,
        ),
        ("content/views/default.svg", "default image"),
        ("content/assets/a?b#c%20.svg", "caller image"),
        ("content/other.not", "@(id: \"part%one\")\n= Other"),
    ]);
    let site = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    assert!(site.warnings.is_empty(), "{:?}", site.warnings);
    let output = html(&site, "pages/start.html");
    for url in [
        "../views/default.svg",
        "../assets/a%3Fb%23c%2520.svg#one",
        "../assets/a%3Fb%23c%2520.svg?v=2#two",
        "../assets/a%3Fb%23c%2520.svg#three",
        "https://cdn.test/a.svg#remote",
    ] {
        assert!(
            output.contains(&format!("src=\"{url}\"")),
            "{url}: {output}"
        );
    }
    assert!(output.contains("href=\"../other.html#part%25one\""));
    assert_eq!(
        site.files[Path::new("assets/a?b#c%20.svg")],
        b"caller image"
    );
    assert!(!output.contains("/site"));
}

#[test]
fn typed_embeds_do_not_publish_missing_private_or_outside_content_resources() {
    for source in [
        r#"#embed(p"./missing.svg")"#,
        r#"#embed(p"./private.notc")"#,
        r#"#embed("../outside.svg")"#,
        r#"#embed(p"./other.not", fragment: "missing")"#,
    ] {
        let result = Builder::default().build(
            &mut vault(&[
                ("content/index.not", source),
                ("content/private.notc", "private module"),
                ("content/other.not", "= Other"),
                ("outside.svg", "not published"),
            ]),
            &SiteConfig::default(),
        );
        assert!(result.is_err(), "{source}");
    }
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
        ("theme/templates/home.tera", "theme"),
        ("theme/templates/part.tera", "{{site.data.example.name}}"),
        ("theme/static/style.css", "theme css"),
        ("static/style.css", "site css"),
        ("data/example.toml", "name='Example'\ncode='**strong**'"),
        (
            "templates/home.tera",
            "{{page.title}}|{% include 'part' %}|{{url(path='style.css')}}|{% for path, dir in site.directories %}{{dir.path}};{% endfor %}|{{markdown(source=site.data.example.code) | safe}}|{{page.html | safe}}",
        ),
    ]);
    let config =
        toml::from_str::<SiteConfig>("base_url='https://example.com/project'\ntheme='theme'")
            .unwrap();
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
fn shared_template_context_keeps_page_directory_and_root_lookups_isolated() {
    let mut vault = vault(&[
        ("content/index.not", "= Home"),
        ("content/items/a.not", "@!(id: \"a\")\n= A"),
        ("content/items/b.not", "@!(id: \"b\")\n= B"),
        ("content/other/c.not", "@!(id: \"c\")\n= C"),
        (
            "data/items.json",
            r#"{"a":"Apple & pear","b":"Berry","c":"Cherry"}"#,
        ),
        (
            "templates/page.tera",
            "{{page.title}}|{{directory.path}}|{% for sibling in directory.pages %}{{sibling.title}};{% endfor %}|{% if page.attrs.id %}{{site.data.items[page.attrs.id]}}/{{page.title}}{% endif %}|{% for other in site.pages %}{{other.title}};{% endfor %}|{{assets.highlight}}",
        ),
    ]);
    let site = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    assert!(html(&site, "items/a.html").starts_with("A|items|A;B;|Apple &amp; pear/A|"));
    assert!(html(&site, "items/b.html").starts_with("B|items|A;B;|Berry/B|"));
    assert!(html(&site, "other/c.html").starts_with("C|other|C;|Cherry/C|"));
    for path in ["items/a.html", "items/b.html", "other/c.html"] {
        assert!(html(&site, path).ends_with("../_notist/highlight.css"));
    }
    assert!(html(&site, "index.html").ends_with("_notist/highlight.css"));
}

#[test]
fn shared_context_preserves_page_bodies_directory_indexes_and_nested_data() {
    let mut vault = vault(&[
        ("content/index.not", "= Home"),
        ("content/folder/index.not", "= Folder"),
        ("content/folder/leaf.not", "= Leaf"),
        ("content/empty/asset.txt", "asset"),
        (
            "data/values.json",
            r#"{"nested":{"enabled":true,"number":42,"text":"<unsafe>"},"list":["one","two"]}"#,
        ),
        (
            "templates/page.tera",
            "{{page.title}}|{{site.directories[''].index.title}}|{{site.directories.folder.index.title}}|{% for sibling in site.directories.folder.pages %}{{sibling.title}};{% endfor %}|{{directory.path}}/{% if directory.index %}{{directory.index.title}}{% endif %}|{% for other in site.pages %}{% if other.title == page.title %}{{other.html | safe}}/{{other.headings[0].anchor}}/{{other.route.url}}{% endif %}{% endfor %}|{{site.data.values.nested.enabled}}/{{site.data.values.nested.number}}/{{site.data.values.nested.text}}/{{site.data.values.list[1]}}|{{site.directories.empty.name}}",
        ),
    ]);
    let site = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    for (path, title, directory, heading) in [
        ("index.html", "Home", "/Home", "home"),
        ("folder/index.html", "Folder", "folder/Folder", "folder"),
        ("folder/leaf.html", "Leaf", "folder/Folder", "leaf"),
    ] {
        let output = html(&site, path);
        assert!(
            output.starts_with(&format!("{title}|Home|Folder|Leaf;|{directory}|")),
            "{output}"
        );
        assert!(
            output.contains(&format!("<h1 id=\"{heading}\">{title}</h1>"))
                && output.contains(&format!("</section>/{heading}/")),
            "{output}"
        );
        assert!(
            output.ends_with("|true/42/&lt;unsafe&gt;/two|empty"),
            "{output}"
        );
    }
}

#[test]
fn tera_inheritance_and_native_expressions_keep_html_escaped() {
    let mut vault = vault(&[
        ("content/index.not", "@!(title: \"<Home> & title\")\n= Body"),
        (
            "templates/page.tera",
            "{% extends 'layouts/base' %}{% block body %}{% if page.title == '<Home> & title' and page.title is starting_with(pat='<') %}{{page.title ~ '!'}}{% endif %}|{{markdown(source='**bold**')}}|{{markdown(source='**bold**') | safe}}|{{url(path='next.html?a=1&b=2')}}|{{page.html | safe}}{% endblock body %}",
        ),
        (
            "templates/layouts/base.tera",
            "<main>{% block body %}base{% endblock body %}</main>",
        ),
    ]);
    let site = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    let output = html(&site, "index.html");
    assert!(
        output.starts_with("<main>&lt;Home&gt; &amp; title!|"),
        "{output}"
    );
    assert!(output.contains("&lt;strong&gt;bold&lt;/strong&gt;"));
    assert!(output.contains("<strong>bold</strong>"));
    assert!(output.contains("next.html?a=1&amp;b=2"));
    assert!(output.contains("<h1 id=\"body\">Body</h1>"));
}

#[test]
fn tera_errors_and_unsafe_helper_urls_abort_publication() {
    for template in [
        "{{page.missing}}",
        "{% if page.title %}unclosed",
        "{{url(path='javascript:alert(1)')}}",
        "{{url()}}",
        "{{markdown()}}",
        "{% include 'missing' %}",
    ] {
        let result = Builder::default().build(
            &mut vault(&[
                ("content/index.not", "= Home"),
                ("templates/page.tera", template),
            ]),
            &SiteConfig::default(),
        );
        assert!(result.is_err(), "{template}");
    }
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
            "templates/page.tera",
            "<link rel=\"stylesheet\" href=\"{{assets.highlight}}\">{{page.html | safe}}{{markdown(source=site.data.code.source) | safe}}",
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
    assert!(page.contains("href=\"_notist/highlight.css\""));
    assert_eq!(
        html(&site, "_notist/highlight.css"),
        notist_html::highlight::CSS
    );
}

#[test]
fn default_template_loads_highlight_styles_relative_to_nested_pages() {
    let mut vault = vault(&[(
        "content/guide/start.md",
        "```toml\n[book]\ntitle = \"Example\"\n```",
    )]);
    let site = Builder::default()
        .build(&mut vault, &SiteConfig::default())
        .unwrap();
    let page = html(&site, "guide/start.html");
    assert!(page.contains("href=\"../_notist/highlight.css\""));
    assert!(page.contains("class=\"language-toml\"><span class="));
}
