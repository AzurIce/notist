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
    assert!(html_text.contains("<section id=\"potion\"><h1>Potion</h1>"), "html: {html_text}");
    assert!(html_text.contains("<strong>Notist</strong>"), "html: {html_text}");
    assert!(
        html_text.contains("<code class=\"language-rust\"><span class=\"z-source z-rust\">"),
        "html: {html_text}"
    );
    assert!(html_out_dir.join("pic.svg").is_file(), "local image should be published");
}

#[test]
#[cfg(unix)]
fn serve_rebuilds_live_on_external_notc_and_hidden_data_file_changes() {
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn http_get(address: &str, path: &str) -> Option<String> {
        let mut stream = TcpStream::connect(address).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .ok()?;
        let mut out = String::new();
        stream.read_to_string(&mut out).ok()?;
        Some(out)
    }

    fn wait_for_body(address: &str, path: &str, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(6);
        let mut last = String::new();
        while Instant::now() < deadline {
            if let Some(resp) = http_get(address, path) {
                if resp.contains(needle) {
                    return resp;
                }
                last = resp;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("preview at {path} did not contain `{needle}` in time; last:\n{last}");
    }

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write(root, "Notist.toml", "[package]\nname = \"live-demo\"\n[site]\n");
    write(
        root,
        "modules/ui.notc",
        r#"
fn banner(txt_path: String, json_path: String) -> Block {
    let raw = read_text(txt_path);
    let data = read_json(json_path);
    let tag = data.get("tag").as_string("none");
    para(text("V1:" + raw + ":" + tag))
}
"#,
    );
    write(root, "content/.data/value.txt", "alpha");
    write(root, "content/.data/value.json", r#"{"tag":"j1"}"#);
    write(
        root,
        "content/index.md",
        r#"# Live Demo

Intro text.

#import "../modules/ui.notc" as ui

#ui.banner("./.data/value.txt", "./.data/value.json")
"#,
    );

    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let address = format!("127.0.0.1:{port}");
    let child = Command::new(notist_bin())
        .current_dir(root)
        .args(["serve", "--address", &address, "--no-clear-screen"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let _guard = ChildGuard(child);

    // 1. Initial build serves content from external `.notc` + hidden `.data/*` files.
    wait_for_body(&address, "/", "V1:alpha:j1");

    // 2. Editing external `.notc` module triggers hot rebuild.
    write(
        root,
        "modules/ui.notc",
        r#"
fn banner(txt_path: String, json_path: String) -> Block {
    let raw = read_text(txt_path);
    let data = read_json(json_path);
    let tag = data.get("tag").as_string("none");
    para(text("V2:" + raw + ":" + tag))
}
"#,
    );
    wait_for_body(&address, "/", "V2:alpha:j1");

    // 3. Editing hidden `.data/value.txt` (`read_text`) and `.data/value.json` (`read_json`) triggers hot rebuild.
    write(root, "content/.data/value.txt", "beta");
    write(root, "content/.data/value.json", r#"{"tag":"j2"}"#);
    wait_for_body(&address, "/", "V2:beta:j2");
}
