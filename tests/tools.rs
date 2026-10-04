#![cfg(not(target_arch = "wasm32"))]
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

#[test]
fn cli_discovers_packages_inspects_modules_and_reports_config_failures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let document = root.join("examples/plugins/document.not");
    let query = Command::new(env!("CARGO_BIN_EXE_notist"))
        .args(["query"])
        .arg(&document)
        .arg("function:widgets::panel")
        .output()
        .unwrap();
    assert!(query.status.success());
    let result: Value = serde_json::from_slice(&query.stdout).unwrap();
    assert_eq!(result.as_array().unwrap().len(), 2);
    let module = Command::new(env!("CARGO_BIN_EXE_notist"))
        .arg("json")
        .arg(root.join("examples/plugins/packages/mermaid/lib.notc"))
        .output()
        .unwrap();
    assert!(module.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&module.stdout).unwrap()["tree"]["kind"],
        "Module"
    );
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("Notist.toml");
    std::fs::write(&config, "[dependencies]\nbroken = {path = 'absent'}").unwrap();
    for command in ["check", "core", "json", "query", "html"] {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_notist"));
        cmd.arg(command).arg(&document).arg("--config").arg(&config);
        if command == "query" {
            cmd.arg("level:block");
        }
        let result = cmd.output().unwrap();
        assert!(!result.status.success(), "{command}");
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(
            error.contains("Notist.toml") && error.contains("cannot load"),
            "{error}"
        );
    }
}

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
}
impl Lsp {
    fn new() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_notist"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let (sender, messages) = channel();
        std::thread::spawn(move || {
            loop {
                let mut length = None;
                loop {
                    let mut line = String::new();
                    if stdout.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.strip_prefix("Content-Length:") {
                        length = value.trim().parse::<usize>().ok();
                    }
                }
                let Some(length) = length else {
                    return;
                };
                let mut body = vec![0; length];
                if stdout.read_exact(&mut body).is_err() {
                    return;
                }
                if sender.send(serde_json::from_slice(&body).unwrap()).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            stdin,
            messages,
        }
    }
    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.stdin.flush().unwrap();
    }
    fn until(&self, predicate: impl Fn(&Value) -> bool) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let message = self
                .messages
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("language server response");
            if predicate(&message) {
                return message;
            }
        }
    }
}
impl Drop for Lsp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn language_server_uses_declarations_unsaved_overlays_and_clears_source_errors() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("widgets")).unwrap();
    std::fs::write(
        temp.path().join("Notist.toml"),
        "[dependencies]\nwidgets = {path = './widgets'}\n",
    )
    .unwrap();
    let module = temp.path().join("widgets/lib.notc");
    let source = "fn panel(title: String)[children: Content] -> Content;";
    std::fs::write(&module, source).unwrap();
    let document = temp.path().join("document.not");
    let uri = tower_lsp::lsp_types::Url::from_file_path(&document)
        .unwrap()
        .to_string();
    let module_uri = tower_lsp::lsp_types::Url::from_file_path(&module)
        .unwrap()
        .to_string();
    let mut server = Lsp::new();
    server.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}));
    server.until(|message| message["id"] == 1);
    server.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"notist","version":1,"text":"#widgets::panel(\"Title\")[body]"}}}));
    let diagnostics = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics" && message["params"]["uri"] == uri
    });
    assert!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    server.send(json!({"jsonrpc":"2.0","id":2,"method":"textDocument/definition","params":{"textDocument":{"uri":uri},"position":{"line":0,"character":5}}}));
    let definition = server.until(|message| message["id"] == 2);
    assert_eq!(definition["result"]["uri"], module_uri);
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":module_uri,"languageId":"notc","version":1,"text":"fn panel(title: Unknown) -> Content;"}}}));
    let invalid = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == module_uri
            && !message["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
    });
    assert!(
        invalid["params"]["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("unknown")
    );
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":module_uri,"version":2},"contentChanges":[{"text":source}]}}));
    let valid = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == module_uri
            && message["params"]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
    });
    assert!(
        valid["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    server.send(json!({"jsonrpc":"2.0","id":3,"method":"textDocument/documentSymbol","params":{"textDocument":{"uri":module_uri}}}));
    assert_eq!(
        server.until(|message| message["id"] == 3)["result"][0]["name"],
        "panel"
    );
}
