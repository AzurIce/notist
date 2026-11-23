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
}
