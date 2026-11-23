use notist::{MemoryResources, Vault};
use notist_ssg::{Builder, Error, SiteConfig};
use std::path::Path;

fn vault(files: &[(&str, &str)]) -> Vault<MemoryResources> {
    let mut resources = MemoryResources::new("/site");
    for (path, text) in files {
        resources.insert(path, *text);
    }
    Vault::new(resources)
}

fn html(output: &notist_ssg::BuildOutput, path: &str) -> String {
    String::from_utf8(output.files[Path::new(path)].clone()).unwrap()
}

#[test]
fn builds_full_site_with_notc_modules_generics_methods_links_and_images() {
    let ui_module = r#"
fn decorate<C: Type>(content: C) -> C
where C == Inline || C == Block
{
    match C {
        Inline => strong[⚡ #content],
        Block  => callout("tip", content),
        _      => compile_error("unreachable"),
    }
}

fn Block.banner(self) -> Block {
    callout("banner", self)
}
"#;
    let index_doc = r#"
@!(title: "Notist Home")
#import "../lib/ui.notc" as ui

@(id: "welcome")
= Welcome to #ui.decorate[Notist]

Read [the architecture guide](./guide.not#arch) or view ![logo](./logo.svg).

#ui.decorate[
    Powered by the new statically typed `notc` engine.
]

```notc
fn id<T: Type>(x: T) -> T { x }
```
"#;
    let guide_doc = r#"
#import "../lib/ui.notc" as ui

= Guide

@(id: "arch")
== Architecture

- Single-node immutable content trees
- Explicit `sectionize()` and bottom-up rewriting

> Zero runtime surprises.

---

#ui.decorate[
    Back to [Home](./index.not#welcome).
]
"#;

    let mut v = vault(&[
        ("lib/ui.notc", ui_module),
        ("content/index.not", index_doc),
        ("content/guide.not", guide_doc),
        ("content/logo.svg", "<svg/>"),
    ]);

    let mut config = SiteConfig::default();
    config.title = "Notist Docs".into();
    config.directory_urls = true;
    config.copy_content_assets = false;

    let output = Builder::default()
        .build(&mut v, &config)
        .expect("site should build cleanly with notc");

    assert!(output.warnings.is_empty(), "warnings: {:?}", output.warnings);
    assert_eq!(output.pages.len(), 2);
    // Referenced image asset was copied automatically even with `copy_content_assets = false`
    assert!(output.files.contains_key(Path::new("logo.svg")));

    let index_html = html(&output, "index.html");
    assert!(index_html.contains("<title>Notist Home · Notist Docs</title>"));
    assert!(index_html.contains(r#"<h1 id="welcome">Welcome to <strong>⚡ Notist</strong></h1>"#));
    assert!(index_html.contains(r#"href="guide/#arch""#));
    assert!(index_html.contains(r#"<img src="logo.svg" alt="logo">"#));
    assert!(index_html.contains(r#"<aside class="callout callout-tip"><p>Powered by the new statically typed <code>notc</code> engine.</p></aside>"#));
    assert!(index_html.contains(r#"class="language-notc""#));

    let guide_html = html(&output, "guide/index.html");
    assert!(guide_html.contains(r#"<h2 id="arch">Architecture</h2>"#));
    assert!(guide_html.contains(r#"<ul><li>Single-node immutable content trees</li><li>Explicit <code>sectionize()</code> and bottom-up rewriting</li></ul>"#));
    assert!(guide_html.contains(r#"<blockquote><p>Zero runtime surprises.</p></blockquote>"#));
    assert!(guide_html.contains("<hr>"));
    assert!(guide_html.contains(r#"href="../#welcome""#));
}

#[test]
fn rejects_static_type_error_in_notc_module_or_document_during_site_build() {
    let ui_module = "fn need_inline(x: Inline) -> Inline { strong(x) }";
    let bad_doc = r#"
#import "../lib/ui.notc" as ui

= Broken Page

#ui.need_inline[
    Passing a Block to an Inline parameter fails statically!
]
"#;
    let mut v = vault(&[
        ("lib/ui.notc", ui_module),
        ("content/index.not", bad_doc),
    ]);
    let err = Builder::default()
        .build(&mut v, &SiteConfig::default())
        .expect_err("type error must fail site build");
    let Error::Diagnostics(diags) = err else {
        panic!("expected Diagnostics error, got {err:?}");
    };
    assert_eq!(diags.len(), 1);
    assert!(diags[0].path.ends_with("content/index.not"));
    assert!(diags[0].diagnostic.message.contains("expected `Inline`, found `Block`"));
}

#[test]
fn ssg_loads_decoupled_web_components_by_convention_from_extern_notc_declarations() {
    let lib = r#"
fn badge(label: String, count: Int) -> Inline;
fn panel(title: String, body: Block) -> Block;
fn fallback_box(body: Block) -> Block;
"#;
    let index_doc = r#"
#import "../components/lib.notc" as ui

= Demo #ui.badge("new", 3)

#ui.panel("Details")[
    Inside a web component with $a^2 + b^2 = c^2$.
]

#ui.fallback_box[
    No JS file required: renders semantic fallback.
]
"#;

    let mut v = vault(&[
        ("components/lib.notc", lib),
        ("components/badge.js", "export default class Badge extends HTMLElement {}"),
        ("components/panel/index.js", "export default class Panel extends HTMLElement {}"),
        ("components/panel/style.js", "export const css = '';"),
        ("content/index.not", index_doc),
    ]);

    let output = Builder::default()
        .build(&mut v, &SiteConfig::default())
        .expect("site with decoupled web components should build");

    let index_html = html(&output, "index.html");
    // Web components with JS files get custom element tags + registration script
    assert!(index_html.contains("<notist-components-badge "));
    assert!(index_html.contains("<notist-components-panel "));
    assert!(index_html.contains(r#"<script type="module" src="_notist/pages/index.js"></script>"#));
    // Custom node without JS file renders semantic fallback without failing
    assert!(index_html.contains(r#"class="notist-component notist-fallback_box""#));
    // Math rendered cleanly inside component children
    assert!(index_html.contains(r#"<span class="notist-math">a^2 + b^2 = c^2</span>"#));

    // Published component JS files
    assert!(output.files.contains_key(Path::new("_notist/components/components/badge.js")));
    assert!(output.files.contains_key(Path::new("_notist/components/components/panel/index.js")));
    assert!(output.files.contains_key(Path::new("_notist/components/components/panel/style.js")));
    let reg_js = html(&output, "_notist/pages/index.js");
    assert!(reg_js.contains("customElements.define(\"notist-components-badge\""));
    assert!(reg_js.contains("customElements.define(\"notist-components-panel\""));
    // Web component attributes include notist-<prop> and notist-content for modules/* compatibility
    assert!(index_html.contains(r#"notist-content="inline""#));
    assert!(index_html.contains(r#"notist-label="new""#));
    assert!(index_html.contains(r#"notist-count="3""#));
}

#[test]
fn ssg_routes_math_nodes_to_configured_math_component_and_supports_collapsed_plain_summary() {
    let mut v = vault(&[
        ("math/index.js", "export default class MathEl extends HTMLElement {}"),
        ("math/helper.js", "export const ok = true;"),
        (
            "content/SUMMARY.not",
            "= Book\n\n- @(collapsed: true)Section Group\n  - [Intro](./intro.md)\n",
        ),
        (
            "content/intro.md",
            "@!(title: \"Math Intro\")\n\n# Math Intro\n\nInline $x + 1$ and block:\n\n$$\ny = x^2\n$$\n",
        ),
    ]);

    let mut config = SiteConfig::default();
    config.math_component = Some("math/index.js".into());

    let output = Builder::default()
        .build(&mut v, &config)
        .expect("site with math_component and collapsed plain summary should build");

    let intro_html = html(&output, "intro.html");
    assert!(intro_html.contains("<title>Math Intro · Site</title>"));
    assert!(intro_html.contains(r#"<notist-math notist-content="inline" notist-text="x + 1"></notist-math>"#));
    assert!(intro_html.contains(r#"<notist-math notist-content="block" notist-text="y = x^2"></notist-math>"#));
    assert!(output.files.contains_key(Path::new("_notist/components/math/index.js")));
    assert!(output.files.contains_key(Path::new("_notist/components/math/helper.js")));
    let reg_js = html(&output, "_notist/pages/intro.js");
    assert!(reg_js.contains("customElements.define(\"notist-math\""));
}

#[test]
fn ssg_evaluates_data_driven_pages_with_cached_json_host_lists_tables_and_containers() {
    let views = r#"
fn item_page(source: String, idx: Int) -> Block {
    let items = read_json(source);
    let rec = items.at(idx);
    let id = rec.at(0).as_string("");
    let name = rec.at(1).as_string("");
    [
        @(id: "item-detail")#section[
            #heading(1, text(name))
            @(id: "meta", class: "item-meta")#div[
                #para(strong[ID] + text(" " + id))
            ]
        ]
    ]
}

fn catalog_page(source: String) -> Block {
    let page = read_json(source);
    let count = page.get("count").as_int(0);
    let rows = for rec in page.get("items").as_list() {
        let id = rec.at(0).as_string("");
        let name = rec.at(1).as_string("");
        #[
            text(id).with_attr("class", "numeric"),
            link("/items/" + id + ".html", text(name)),
        ]
    };
    [
        = Items

        #para(text(count.to_string() + " total"))

        @(class: "table-scroll")#div[
            #table(#[text("ID"), text("Name")], rows)
        ]

        @(class: "pagination", aria-label: "Catalog Pages")
        Page 1
    ]
}
"#;
    let mut v = vault(&[
        ("modules/views.notc", views),
        ("content/.data/shard.json", r#"[["1","Gil"],["2","Fire Shard"]]"#),
        ("content/.data/catalog.json", r#"{"count":2,"items":[["1","Gil"],["2","Fire Shard"]]}"#),
        ("content/items/index.not", "#import \"../../modules/views.notc\" as views;\n#views.catalog_page(\"../.data/catalog.json\")\n"),
        ("content/items/1.not", "#import \"../../modules/views.notc\" as views;\n#views.item_page(\"../.data/shard.json\", 0)\n"),
        ("content/items/2.not", "#import \"../../modules/views.notc\" as views;\n#views.item_page(\"../.data/shard.json\", 1)\n"),
    ]);

    let output = Builder::default()
        .build(&mut v, &SiteConfig::default())
        .expect("data-driven site builds cleanly");

    assert!(output.warnings.is_empty(), "warnings: {:?}", output.warnings);
    assert_eq!(output.pages.len(), 3);

    let catalog = html(&output, "items/index.html");
    assert!(catalog.contains("<p>2 total</p>"));
    assert!(catalog.contains(r#"<div class="table-scroll"><table><thead><tr><th>ID</th><th>Name</th></tr></thead><tbody><tr><td class="numeric">1</td><td><a href="1.html">Gil</a></td></tr><tr><td class="numeric">2</td><td><a href="2.html">Fire Shard</a></td></tr></tbody></table></div>"#));
    assert!(catalog.contains(r#"<p class="pagination" aria-label="Catalog Pages">Page 1</p>"#));

    let item2 = html(&output, "items/2.html");
    assert!(item2.contains(r#"<section id="item-detail">"#));
    assert!(item2.contains(r#"<h1 id="fire-shard">Fire Shard</h1>"#));
    assert!(item2.contains(r#"<div id="meta" class="item-meta"><p><strong>ID</strong> 2</p></div>"#));
}
