use std::fs;
use std::path::Path;
use std::process::Command;

fn notist_bin() -> &'static str {
    env!("CARGO_BIN_EXE_notist")
}

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

#[test]
fn cli_check_cst_core_json_query_and_html_run_end_to_end_on_notc() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    write(
        root,
        "lib/ui.notc",
        r#"
fn badge(label: String) -> Inline {
    strong(text(label))
}

fn item_title(source: String) -> Block {
    let data = read_json(source);
    let name = data.get("name").as_string("unknown");
    heading(1, text(name))
}

fn main() -> Int {
    40 + 2
}
"#,
    );
    write(root, "data/item.json", r#"{"name": "Potion"}"#);
    write(root, "docs/pic.svg", "<svg/>");
    write(
        root,
        "docs/page.not",
        r#"@!(title: "CLI Demo")
#import "../lib/ui.notc" as ui

#ui.item_title("../data/item.json")

@(id: "intro", tags: "alpha, beta")
Hello #ui.badge("Notist") ![logo](./pic.svg)

```rust,ignore
fn main() {}
```
"#,
    );
    write(
        root,
        "docs/readme.md",
        r#"#import "../lib/ui.notc" as ui

# Markdown #ui.badge("OK")

```rust,no_run
let x = 1;
```
"#,
    );

    // 1. `notist check` on .not, .md, and .notc
    for rel in ["docs/page.not", "docs/readme.md", "lib/ui.notc"] {
        let out = Command::new(notist_bin())
            .args(["check", root.join(rel).to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "check {rel} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    // 2. `notist cst` prints parsed notc Module AST
    let cst = Command::new(notist_bin())
        .args(["cst", root.join("docs/page.not").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(cst.status.success());
    let cst_out = String::from_utf8(cst.stdout).unwrap();
    assert!(cst_out.contains("Module"), "cst: {cst_out}");

    // 3. `notist core --eval` evaluates document (with read_json) and .notc `main`
    let core_doc = Command::new(notist_bin())
        .args(["--eval", "core", root.join("docs/page.not").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(core_doc.status.success());
    let core_doc_out = String::from_utf8(core_doc.stdout).unwrap();
    assert!(core_doc_out.contains("Potion"), "core doc: {core_doc_out}");

    let core_mod = Command::new(notist_bin())
        .args(["--eval", "core", root.join("lib/ui.notc").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(core_mod.status.success());
    let core_mod_out = String::from_utf8(core_mod.stdout).unwrap();
    assert!(core_mod_out.contains("Int(\n    42"), "core mod: {core_mod_out}");

    // 4. `notist json` outputs valid JSON with diagnostics, symbols, and evaluated core tree
    let json_cmd = Command::new(notist_bin())
        .args(["json", root.join("docs/page.not").to_str().unwrap()])
        .output()
        .unwrap();
    assert!(json_cmd.status.success());
    let json_val: serde_json::Value = serde_json::from_slice(&json_cmd.stdout).unwrap();
    assert_eq!(json_val["diagnostics"].as_array().unwrap().len(), 0);
    assert_eq!(json_val["core"]["ctor"], "fragment");

    // 5. `notist query` selects nodes by id, tag, and ctor
    let query_id = Command::new(notist_bin())
        .args(["query", root.join("docs/page.not").to_str().unwrap(), "id:intro"])
        .output()
        .unwrap();
    assert!(query_id.status.success());
    let matches: serde_json::Value = serde_json::from_slice(&query_id.stdout).unwrap();
    assert_eq!(matches.as_array().unwrap().len(), 1);
    assert_eq!(matches[0]["ctor"], "paragraph");

    let query_ctor = Command::new(notist_bin())
        .args(["query", root.join("docs/readme.md").to_str().unwrap(), "ctor:strong"])
        .output()
        .unwrap();
    assert!(query_ctor.status.success());
    let md_matches: serde_json::Value = serde_json::from_slice(&query_ctor.stdout).unwrap();
    assert_eq!(md_matches.as_array().unwrap().len(), 1);

    // 6. `notist html` evaluates read_json, highlights `rust,ignore`, and copies local images
    let html_out_dir = root.join("out-html");
    let html_cmd = Command::new(notist_bin())
        .args([
            "html",
            root.join("docs/page.not").to_str().unwrap(),
            "--out-dir",
            html_out_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        html_cmd.status.success(),
        "html failed: {}",
        String::from_utf8_lossy(&html_cmd.stderr)
    );
    let html_text = fs::read_to_string(html_out_dir.join("index.html")).unwrap();
    assert!(html_text.contains("<h1 id=\"potion\">Potion</h1>"), "html: {html_text}");
    assert!(html_text.contains("<strong>Notist</strong>"), "html: {html_text}");
    assert!(
        html_text.contains("<code class=\"language-rust\"><span class=\"z-source z-rust\">"),
        "html: {html_text}"
    );
    assert!(html_out_dir.join("pic.svg").is_file(), "local image should be published");
}
