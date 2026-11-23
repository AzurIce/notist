#![cfg(all(not(target_arch = "wasm32"), feature = "lsp"))]
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
}

impl Lsp {
    fn new(cwd: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_notist"))
            .arg("lsp")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn notist lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(msg) = read_message(&mut reader) {
                if tx.send(msg).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            messages: rx,
        }
    }

    fn initialized(cwd: &Path) -> Self {
        let mut server = Self::new(cwd);
        let root_uri = url::Url::from_directory_path(cwd).unwrap().to_string();
        let init = server.request(
            1,
            "initialize",
            json!({"rootUri": root_uri, "capabilities": {}}),
        );
        assert_eq!(
            init["result"]["capabilities"]["completionProvider"]["triggerCharacters"],
            json!(["#", "."])
        );
        server.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
        server
    }

    fn send(&mut self, msg: Value) {
        let body = msg.to_string();
        write!(
            self.stdin,
            "Content-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
        .unwrap();
        self.stdin.flush().unwrap();
    }

    fn until(&mut self, mut pred: impl FnMut(&Value) -> bool) -> Value {
        loop {
            let msg = self
                .messages
                .recv_timeout(Duration::from_secs(10))
                .expect("LSP response timeout");
            if pred(&msg) {
                return msg;
            }
        }
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        self.until(|msg| msg["id"] == id)
    }

    fn diagnostics(&mut self, uri: &str, version: i32) -> Value {
        self.until(|msg| {
            msg["method"] == "textDocument/publishDiagnostics"
                && msg["params"]["uri"] == uri
                && msg["params"]["version"] == version
        })
    }

    fn open(&mut self, uri: &str, version: i32, text: &str) -> Value {
        self.send(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "languageId": if uri.ends_with(".notc") { "notc" } else { "notist" },
                    "version": version,
                    "text": text
                }
            }
        }));
        self.diagnostics(uri, version)
    }

    fn change(&mut self, uri: &str, version: i32, text: &str) -> Value {
        self.send(json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didChange",
            "params": {
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }]
            }
        }));
        self.diagnostics(uri, version)
    }
}

impl Drop for Lsp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut len = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            len = rest.trim().parse::<usize>().ok();
        }
    }
    let len = len?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

#[test]
fn completions_follow_imports_unsaved_overlays_and_utf16_edits() {
    let temp = tempfile::tempdir().unwrap();
    let module = temp.path().join("lib.notc");
    std::fs::write(&module, "fn panel(title: Inline) -> Inline { strong(title) }").unwrap();
    let module_uri = url::Url::from_file_path(&module).unwrap().to_string();
    let doc_uri = url::Url::from_file_path(temp.path().join("doc.not"))
        .unwrap()
        .to_string();

    let mut server = Lsp::initialized(temp.path());
    server.open(
        &doc_uri,
        1,
        "#import \"./lib.notc\" as widgets\n😀 #widgets.panel[Title]",
    );

    // Emoji 😀 is 2 UTF-16 code units; `😀 #widgets.pa` -> character 14 is inside `panel`
    let items = server.request(
        2,
        "textDocument/completion",
        json!({"textDocument":{"uri":doc_uri},"position":{"line":1,"character":14}}),
    );
    let panel = items["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["label"] == "panel")
        .expect("expected `panel` completion");
    assert_eq!(panel["kind"], 3); // FUNCTION
    assert!(panel["detail"].as_str().unwrap().contains("title: Inline"));
    assert_eq!(
        panel["textEdit"],
        json!({
            "range": {"start":{"line":1,"character":12},"end":{"line":1,"character":17}},
            "newText": "panel"
        })
    );

    // Dot completion after `#widgets.`
    server.change(&doc_uri, 2, "#import \"./lib.notc\" as widgets\n#widgets.");
    let members = server.request(
        3,
        "textDocument/completion",
        json!({"textDocument":{"uri":doc_uri},"position":{"line":1,"character":9}}),
    );
    let arr = members["result"].as_array().unwrap();
    assert!(arr.iter().any(|item| item["label"] == "panel"));
    assert!(!arr.iter().any(|item| item["label"] == "strong"));

    // Unsaved overlay in `lib.notc` immediately replaces `panel` with `card`
    server.open(&module_uri, 1, "fn card(body: Block) -> Block { callout(\"tip\", body) }");
    let updated = server.request(
        4,
        "textDocument/completion",
        json!({"textDocument":{"uri":doc_uri},"position":{"line":1,"character":9}}),
    );
    let arr = updated["result"].as_array().unwrap();
    assert!(arr.iter().any(|item| item["label"] == "card"));
    assert!(!arr.iter().any(|item| item["label"] == "panel"));
}

#[test]
fn type_aware_method_completions_hover_and_goto_def_in_notc() {
    let temp = tempfile::tempdir().unwrap();
    let ui_path = temp.path().join("ui.notc");
    std::fs::write(
        &ui_path,
        r#"fn decorate<C: Type>(content: C) -> C
where C == Inline || C == Block
{
    match C {
        Inline => strong(content),
        Block  => callout("note", content),
        _      => compile_error("unreachable"),
    }
}

fn Block.banner(self) -> Block {
    callout("banner", self)
}
"#,
    )
    .unwrap();
    let ui_uri = url::Url::from_file_path(&ui_path).unwrap().to_string();
    let main_uri = url::Url::from_file_path(temp.path().join("main.notc"))
        .unwrap()
        .to_string();

    let mut server = Lsp::initialized(temp.path());
    let main_src = "import \"./ui.notc\";\nfn run(doc: Block) -> Block {\n    let d = ui.decorate(doc);\n    d.\n}\n";
    server.open(&main_uri, 1, main_src);

    // 1. Hover on `ui.decorate` shows generic signature + where guard
    let hover = server.request(
        2,
        "textDocument/hover",
        json!({"textDocument":{"uri":main_uri},"position":{"line":2,"character":17}}),
    );
    let hover_val = hover["result"]["contents"]["value"].as_str().unwrap();
    assert!(hover_val.contains("fn decorate<C: Type>(content: C) -> C"));
    assert!(hover_val.contains("where C == Inline || C == Block"));

    // 2. Goto definition on `ui.decorate` jumps into `ui.notc` line 0
    let def = server.request(
        3,
        "textDocument/definition",
        json!({"textDocument":{"uri":main_uri},"position":{"line":2,"character":17}}),
    );
    assert_eq!(def["result"]["uri"], ui_uri);
    assert_eq!(def["result"]["range"]["start"]["line"], 0);

    // 3. Dot-completion on `d.` (inferred type `Block`) offers BOTH user method `banner` and built-in `sectionize`
    let methods = server.request(
        4,
        "textDocument/completion",
        json!({"textDocument":{"uri":main_uri},"position":{"line":3,"character":6}}),
    );
    let arr = methods["result"].as_array().unwrap();
    assert!(arr.iter().any(|item| item["label"] == "banner" && item["kind"] == 2));
    assert!(arr.iter().any(|item| item["label"] == "sectionize" && item["kind"] == 2));
    assert!(!arr.iter().any(|item| item["label"] == "rewrite"));
}

#[test]
fn diagnostics_publish_clear_and_symbols_survive_broken_imports() {
    let temp = tempfile::tempdir().unwrap();
    let uri = url::Url::from_file_path(temp.path().join("lib.notc"))
        .unwrap()
        .to_string();
    let mut server = Lsp::initialized(temp.path());

    // Broken import still publishes diagnostics AND preserves document symbols + hover!
    let diags = server.open(
        &uri,
        1,
        "import \"./missing.notc\";\nfn kept(x: Inline) -> Inline { strong(x) }\n",
    );
    let list = diags["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert!(list[0]["message"].as_str().unwrap().contains("missing.notc"));

    let symbols = server.request(
        2,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":uri}}),
    );
    assert_eq!(symbols["result"][0]["name"], "kept");

    let hover = server.request(
        3,
        "textDocument/hover",
        json!({"textDocument":{"uri":uri},"position":{"line":1,"character":4}}),
    );
    assert!(
        hover["result"]["contents"]["value"]
            .as_str()
            .unwrap()
            .contains("fn kept(x: Inline) -> Inline")
    );

    // Fixing the file clears diagnostics
    let cleared = server.change(&uri, 2, "fn kept(x: Inline) -> Inline { strong(x) }\n");
    assert!(cleared["params"]["diagnostics"].as_array().unwrap().is_empty());
}

#[test]
fn document_outline_and_cross_document_anchor_definition() {
    let temp = tempfile::tempdir().unwrap();
    let target_path = temp.path().join("target.not");
    std::fs::write(
        &target_path,
        "= Guide\n\n@(id: \"sec\")\n== Target Section\n\nDetails.\n",
    )
    .unwrap();
    let target_uri = url::Url::from_file_path(&target_path).unwrap().to_string();
    let doc_uri = url::Url::from_file_path(temp.path().join("index.not"))
        .unwrap()
        .to_string();

    let mut server = Lsp::initialized(temp.path());
    server.open(&doc_uri, 1, "= Home\n\nRead [the section](./target.not#sec).\n");

    // Jump from link in `index.not` directly to `== Target Section` (line 3) in `target.not`
    let def = server.request(
        2,
        "textDocument/definition",
        json!({"textDocument":{"uri":doc_uri},"position":{"line":2,"character":8}}),
    );
    assert_eq!(def["result"]["uri"], target_uri);
    assert_eq!(def["result"]["range"]["start"]["line"], 3);

    // Nested heading outline for `target.not`
    server.open(
        &target_uri,
        1,
        "= Guide\n\n@(id: \"sec\")\n== Target Section\n\nDetails.\n",
    );
    let outline = server.request(
        3,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":target_uri}}),
    );
    assert_eq!(outline["result"][0]["name"], "Guide");
    assert_eq!(outline["result"][0]["children"][0]["name"], "Target Section");
}
