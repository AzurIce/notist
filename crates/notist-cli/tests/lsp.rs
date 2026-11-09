#![cfg(all(not(target_arch = "wasm32"), feature = "lsp"))]
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<Value>,
}

#[test]
fn language_server_reports_and_clears_outside_vault_references() {
    let temp = tempfile::tempdir().unwrap();
    let uri = url::Url::from_file_path(temp.path().join("page.md"))
        .unwrap()
        .to_string();
    let mut server = Lsp::new(temp.path());
    server.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}));
    server.until(|message| message["id"] == 1);
    server.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"markdown","version":1,"text":"[outside](../outside.not) ![image](../outside.svg)"}}}));
    let invalid = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics" && message["params"]["uri"] == uri
    });
    let diagnostics = invalid["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 2);
    assert!(diagnostics.iter().all(|diagnostic| {
        diagnostic["message"]
            .as_str()
            .unwrap()
            .contains("outside Vault root")
    }));
    assert_eq!(diagnostics[0]["range"]["start"]["character"], 0);
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"[inside](sub/../inside.not) ![remote](https://example.test/image.svg)"}]}}));
    let valid = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics" && message["params"]["uri"] == uri
    });
    assert!(
        valid["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
impl Lsp {
    fn initialized(root: &std::path::Path) -> Self {
        let mut server = Self::new(root);
        server.request(1, "initialize", json!({"capabilities": {}}));
        server.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
        server
    }

    fn request(&mut self, id: i32, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        self.until(|message| message["id"] == id)
    }

    fn open(&mut self, uri: &str, version: i32, source: &str) -> Value {
        self.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"notist","version":version,"text":source}}}));
        self.diagnostics(uri, version)
    }

    fn change(&mut self, uri: &str, version: i32, source: &str) -> Value {
        self.send(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":version},"contentChanges":[{"text":source}]}}));
        self.diagnostics(uri, version)
    }

    fn diagnostics(&self, uri: &str, version: i32) -> Value {
        self.until(|message| {
            message["method"] == "textDocument/publishDiagnostics"
                && message["params"]["uri"] == uri
                && message["params"]["version"] == version
        })
    }

    fn new(root: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_notist"))
            .current_dir(root)
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
    std::fs::write(temp.path().join("Notist.toml"), "").unwrap();
    std::fs::write(temp.path().join("widgets/Notist.toml"), "").unwrap();
    std::fs::write(
        temp.path().join("lib.notc"),
        "import \"widgets/lib.notc\" as widgets;",
    )
    .unwrap();
    let module = temp.path().join("widgets/lib.notc");
    let source = "fn panel(title: String)[children: BlockContent] -> BlockContent;";
    std::fs::write(&module, source).unwrap();
    let document = temp.path().join("document.not");
    let uri = url::Url::from_file_path(&document).unwrap().to_string();
    let module_uri = url::Url::from_file_path(&module).unwrap().to_string();
    let mut server = Lsp::new(temp.path());
    server.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}));
    server.until(|message| message["id"] == 1);
    server.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"notist","version":1,"text":"#import \"widgets/lib.notc\" as widgets;\n#widgets.panel(\"Title\")[ body ]"}}}));
    let diagnostics = server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics" && message["params"]["uri"] == uri
    });
    assert!(
        diagnostics["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    server.send(json!({"jsonrpc":"2.0","id":2,"method":"textDocument/definition","params":{"textDocument":{"uri":uri},"position":{"line":1,"character":5}}}));
    let definition = server.until(|message| message["id"] == 2);
    assert_eq!(definition["result"]["uri"], module_uri);
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":module_uri,"languageId":"notc","version":1,"text":"fn panel(title: Unknown) -> BlockContent;"}}}));
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

#[test]
fn code_diagnostics_publish_to_the_definition_and_do_not_slice_unicode_callers() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("Notist.toml"), "").unwrap();
    let module = temp.path().join("lib.notc");
    std::fs::write(
        &module,
        "fn f() -> InlineContent { compile_error(\"bad\") }",
    )
    .unwrap();
    let module_uri = url::Url::from_file_path(&module).unwrap().to_string();
    let uri = url::Url::from_file_path(temp.path().join("doc.not"))
        .unwrap()
        .to_string();
    let mut server = Lsp::new(temp.path());
    server.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}));
    server.until(|m| m["id"] == 1);
    server.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"notist","version":1,"text":format!("#import \"lib.notc\" as demo;\n#demo.f()  {}", "汉".repeat(30))}}}));
    let invalid = server.until(|m| {
        m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == module_uri
    });
    assert_eq!(invalid["params"]["diagnostics"][0]["message"], "bad");
    assert_eq!(
        invalid["params"]["diagnostics"][0]["range"]["start"]["character"],
        26
    );
    server.send(json!({"jsonrpc":"2.0","id":2,"method":"textDocument/hover","params":{"textDocument":{"uri":uri},"position":{"line":1,"character":6}}}));
    assert!(server.until(|m| m["id"] == 2).get("result").is_some());
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":module_uri,"languageId":"notc","version":1,"text":"fn f() -> InlineContent { [fixed] }"}}}));
    server.until(|m| {
        m["method"] == "textDocument/publishDiagnostics"
            && m["params"]["uri"] == module_uri
            && m["params"]["diagnostics"]
                .as_array()
                .is_some_and(Vec::is_empty)
    });
}

#[test]
fn opening_code_alone_reports_and_clears_body_checking_and_parse_errors() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join("Notist.toml"), "").unwrap();
    std::fs::write(
        temp.path().join("lib.notc"),
        "fn good() -> InlineContent { [good] }",
    )
    .unwrap();
    let uri = url::Url::from_file_path(temp.path().join("lib.notc"))
        .unwrap()
        .to_string();
    let mut server = Lsp::new(temp.path());
    server.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}));
    server.until(|m| m["id"] == 1);
    server.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"notc","version":1,"text":"fn bad() -> InlineContent { compile_error(\"broken body\") }"}}}));
    let invalid = server
        .until(|m| m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri);
    assert!(
        invalid["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["message"] == "broken body")
    );
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":2},"contentChanges":[{"text":"fn generic<C: Type>() -> C { 1 + }"}]}}));
    server.until(|m| {
        m["method"] == "textDocument/publishDiagnostics"
            && m["params"]["uri"] == uri
            && m["params"]["diagnostics"]
                .as_array()
                .is_some_and(|ds| ds.iter().any(|d| d["source"] == "notist:syntax"))
    });
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":3},"contentChanges":[{"text":"fn helper() -> InlineContent { [fixed] } fn good() -> InlineContent { self.helper() }"}]}}));
    server.until(|m| {
        m["method"] == "textDocument/publishDiagnostics"
            && m["params"]["uri"] == uri
            && m["params"]["diagnostics"]
                .as_array()
                .is_some_and(Vec::is_empty)
    });
}

#[test]
fn member_imports_navigate_to_original_definitions_in_every_source_language() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("parts")).unwrap();
    let module = temp.path().join("parts/issue.notc");
    let definition_source = "fn issue() -> InlineContent { [original definition] }";
    std::fs::write(&module, definition_source).unwrap();
    let module_uri = url::Url::from_file_path(&module).unwrap().to_string();
    let mut server = Lsp::new(temp.path());
    server.send(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}));
    server.until(|m| m["id"] == 1);
    server.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}));
    let mut id = 2;
    for extension in ["notc", "not", "md"] {
        for (tail, name) in [(": issue as ticket", "ticket"), (": *", "issue")] {
            let uri = url::Url::from_file_path(temp.path().join(format!("{name}.{extension}")))
                .unwrap()
                .to_string();
            let prefix = if extension == "notc" { "" } else { "#" };
            let call = if extension == "notc" {
                format!("fn f() -> InlineContent {{ {name}() }}")
            } else {
                format!("#{name}()")
            };
            let source = format!("{prefix}import \"parts/issue.notc\"{tail};\n{call}");
            server.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":extension,"version":1,"text":source}}}));
            let diagnostics = server.until(|m| {
                m["method"] == "textDocument/publishDiagnostics" && m["params"]["uri"] == uri
            });
            assert!(
                diagnostics["params"]["diagnostics"]
                    .as_array()
                    .unwrap()
                    .is_empty(),
                "{diagnostics}"
            );
            let character = call.find(name).unwrap();
            for (method, line, character) in [
                ("textDocument/definition", 1, character),
                ("textDocument/hover", 1, character),
                (
                    "textDocument/definition",
                    0,
                    source.find("ticket;").unwrap_or(0),
                ),
            ] {
                if line == 0 && name != "ticket" {
                    continue;
                }
                server.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":{"textDocument":{"uri":uri},"position":{"line":line,"character":character}}}));
                let response = server.until(|m| m["id"] == id);
                if method.ends_with("definition") {
                    assert_eq!(
                        response["result"]["uri"], module_uri,
                        "{extension} {tail}: {response}"
                    );
                    assert_eq!(response["result"]["range"]["start"]["character"], 0);
                } else {
                    assert_eq!(
                        response["result"]["contents"]["value"], definition_source,
                        "{extension} {tail}: {response}"
                    );
                }
                id += 1;
            }
        }
    }
}

#[test]
fn editor_and_cli_share_id_and_same_document_link_checks() {
    let temp = tempfile::tempdir().unwrap();
    let source =
        "@(id: \"same\")#strong[first]\n\n@(id: \"same\")#strong[second]\n\n[missing](#absent)";
    let path = temp.path().join("doc.not");
    std::fs::write(&path, source).unwrap();
    let uri = url::Url::from_file_path(&path).unwrap().to_string();
    let mut server = Lsp::initialized(temp.path());
    let diagnostics = server.open(&uri, 1, source);
    let messages: Vec<_> = diagnostics["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|diagnostic| diagnostic["message"].as_str().unwrap())
        .collect();
    assert_eq!(messages.len(), 2, "{diagnostics}");
    assert!(messages.contains(&"duplicate id `same`"));
    assert!(messages.contains(&"missing item `#absent` in this document"));
    let cli = Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(temp.path())
        .arg("check")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!cli.status.success());
    let stderr = String::from_utf8(cli.stderr).unwrap();
    for message in messages {
        assert!(stderr.contains(message), "{stderr}");
    }
    let valid = server.change(&uri, 2, "@(id: \"same\")#strong[first]\n\n[valid](#same)");
    assert!(
        valid["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn queries_reuse_extension_results_and_edits_and_watches_start_new_batches() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(
        root.join("hook.sh"),
        "cat > \"$1.request\"\nprintf '%s\\n' \"$1\" >> calls\ncat \"$1.response\"\n",
    )
    .unwrap();
    let lowered = notist::Pipeline::default()
        .lower("doc.not", "= Generated\n", Default::default())
        .unwrap();
    let document = notist::extensions::LoweredDocument {
        forest: lowered.forest,
        annotations: lowered.annotations,
        attrs: lowered.module_attrs,
    };
    std::fs::write(
        root.join("before_check.response"),
        json!({"version":1,"document":document}).to_string(),
    )
    .unwrap();
    std::fs::write(root.join("after_check.response"), json!({"version":1,"diagnostics":[{"phase":"Semantic","span":[2,3],"message":"extension diagnostic"}]}).to_string()).unwrap();
    std::fs::write(root.join("after_evaluate.response"), r#"{"version":1}"#).unwrap();
    let config: String = ["before_check", "after_check", "after_evaluate"].iter().map(|stage| {
        format!("[[extensions]]\nkind='command'\ncommand=['sh','hook.sh','{stage}']\nstages=['{stage}']\n")
    }).collect();
    std::fs::write(root.join("Notist.toml"), config).unwrap();
    let uri = url::Url::from_file_path(root.join("doc.not"))
        .unwrap()
        .to_string();
    let mut server = Lsp::initialized(root);
    let invalid = server.open(&uri, 1, "= Original!\n");
    assert!(
        invalid["params"]["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("extension diagnostic")
    );
    for id in 2..8 {
        let outline = server.request(
            id,
            "textDocument/documentSymbol",
            json!({"textDocument":{"uri":uri}}),
        );
        assert_eq!(outline["result"][0]["name"], "Generated");
        let hover = server.request(
            id + 10,
            "textDocument/hover",
            json!({"textDocument":{"uri":uri},"position":{"line":0,"character":4}}),
        );
        assert!(
            hover["result"]["contents"]["value"]
                .as_str()
                .unwrap()
                .contains("Generated"),
            "{hover}"
        );
        server.request(
            id + 20,
            "textDocument/definition",
            json!({"textDocument":{"uri":uri},"position":{"line":0,"character":4}}),
        );
    }
    assert_eq!(
        std::fs::read_to_string(root.join("calls")).unwrap(),
        "before_check\nafter_check\n"
    );
    // A repair observed by the next input batch clears extension errors.
    std::fs::write(root.join("after_check.response"), r#"{"version":1}"#).unwrap();
    let valid = server.change(&uri, 2, "= Modified!\n");
    assert!(
        valid["params"]["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        std::fs::read_to_string(root.join("calls"))
            .unwrap()
            .lines()
            .count(),
        4
    );
    // Old document versions do not rerun hooks or replace the current source.
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didChange","params":{"textDocument":{"uri":uri,"version":1},"contentChanges":[{"text":"= Stale"}]}}));
    server.request(
        40,
        "textDocument/hover",
        json!({"textDocument":{"uri":uri},"position":{"line":0,"character":4}}),
    );
    assert_eq!(
        std::fs::read_to_string(root.join("calls"))
            .unwrap()
            .lines()
            .count(),
        4
    );
    // Only analysis hooks run, even when the config also declares evaluation.
    server.send(json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{"changes":[{"uri":uri,"type":2}]}}));
    server.diagnostics(&uri, 2);
    assert_eq!(
        std::fs::read_to_string(root.join("calls"))
            .unwrap()
            .lines()
            .count(),
        6
    );
    assert!(!root.join("after_evaluate.request").exists());
    let config_uri = url::Url::from_file_path(root.join("Notist.toml"))
        .unwrap()
        .to_string();
    server.open(&config_uri, 1, "");
    let outline = server.request(
        41,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":uri}}),
    );
    assert_eq!(outline["result"][0]["name"], "Modified!");
    assert_eq!(
        std::fs::read_to_string(root.join("calls"))
            .unwrap()
            .lines()
            .count(),
        6
    );
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":config_uri}}}));
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == config_uri
            && message["params"]["version"].is_null()
    });
    let outline = server.request(
        42,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":uri}}),
    );
    assert_eq!(outline["result"][0]["name"], "Generated");
    assert_eq!(
        std::fs::read_to_string(root.join("calls"))
            .unwrap()
            .lines()
            .count(),
        8
    );
}

#[test]
fn links_navigate_labels_and_fragments_using_disk_and_unsaved_targets() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(
        root.join("target.not"),
        "= Disk\n\n@(id: \"anchor\")#strong[Disk target]\n",
    )
    .unwrap();
    let uri = url::Url::from_file_path(root.join("doc.not"))
        .unwrap()
        .to_string();
    let target = url::Url::from_file_path(root.join("target.not"))
        .unwrap()
        .to_string();
    let mut server = Lsp::initialized(root);
    let source = "[target](target.not#anchor)\n\n[mail](mailto:me@example.test)\n\n#link(\"//example.test/path\")[net]\n\n@(id: \"local\")#strong[Local]\n\n[local](#local)\n\n#link(p\"./target.not\", fragment: \"anchor\")[Path]\n";
    server.open(&uri, 1, source);
    let query = |line, character| json!({"textDocument":{"uri":uri},"position":{"line":line,"character":character}});
    let definition = server.request(2, "textDocument/definition", query(0, 2));
    assert_eq!(definition["result"]["uri"], target);
    assert_eq!(definition["result"]["range"]["start"]["line"], 2);
    for (id, line, character) in [(3, 2, 2), (4, 4, 29)] {
        let response = server.request(id, "textDocument/definition", query(line, character));
        assert!(response["result"].is_null(), "line {line}: {response}");
    }
    let definition = server.request(5, "textDocument/definition", query(8, 2));
    assert_eq!(definition["result"]["uri"], uri);
    assert_eq!(definition["result"]["range"]["start"]["line"], 6);
    let path_label = source.lines().nth(10).unwrap().find("Path").unwrap();
    let definition = server.request(9, "textDocument/definition", query(10, path_label));
    assert_eq!(definition["result"]["uri"], target);
    assert_eq!(definition["result"]["range"]["start"]["line"], 2);
    // The overlay wins over disk, including UTF-16 columns before a target.
    server.open(
        &target,
        1,
        "= Unsaved\n\n😀 @(id: \"anchor\")#strong[Unsaved target]\n",
    );
    let definition = server.request(6, "textDocument/definition", query(0, 2));
    assert_eq!(definition["result"]["range"]["start"]["line"], 2);
    assert_eq!(definition["result"]["range"]["start"]["character"], 18);
    server.change(&target, 2, "= Removed anchor\n");
    assert!(server.request(7, "textDocument/definition", query(0, 2))["result"].is_null());
    server.send(json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":target}}}));
    server.until(|message| {
        message["method"] == "textDocument/publishDiagnostics"
            && message["params"]["uri"] == target
            && message["params"]["version"].is_null()
    });
    let definition = server.request(8, "textDocument/definition", query(0, 2));
    assert_eq!(definition["result"]["uri"], target);
    assert_eq!(definition["result"]["range"]["start"]["character"], 15);
}

#[test]
fn navigation_keeps_its_source_revision_until_an_input_event_and_recovers_with_overlays() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let disk = "fn f() -> InlineContent { [disk] }";
    let changed = "fn f() -> InlineContent { [changed] }";
    std::fs::write(root.join("dep.notc"), disk).unwrap();
    let uri = url::Url::from_file_path(root.join("doc.not"))
        .unwrap()
        .to_string();
    let dep = url::Url::from_file_path(root.join("dep.notc"))
        .unwrap()
        .to_string();
    let mut server = Lsp::initialized(root);
    server.open(&uri, 1, "#import \"dep.notc\": f;\n#f()");
    let query = json!({"textDocument":{"uri":uri},"position":{"line":1,"character":1}});
    assert_eq!(
        server.request(2, "textDocument/hover", query.clone())["result"]["contents"]["value"],
        disk
    );
    std::fs::write(root.join("dep.notc"), changed).unwrap();
    assert_eq!(
        server.request(3, "textDocument/hover", query.clone())["result"]["contents"]["value"],
        disk
    );
    server.send(json!({"jsonrpc":"2.0","method":"workspace/didChangeWatchedFiles","params":{"changes":[{"uri":dep,"type":2}]}}));
    server.diagnostics(&uri, 1);
    assert_eq!(
        server.request(4, "textDocument/hover", query.clone())["result"]["contents"]["value"],
        changed
    );
    server.open(&dep, 1, "fn f(value: Unknown) -> InlineContent;");
    assert!(server.request(5, "textDocument/hover", query.clone())["result"].is_null());
    let fixed = "fn f() -> InlineContent { [unsaved fixed] }";
    server.change(&dep, 2, fixed);
    assert_eq!(
        server.request(6, "textDocument/hover", query)["result"]["contents"]["value"],
        fixed
    );
    let definition = server.request(
        7,
        "textDocument/definition",
        json!({"textDocument":{"uri":uri},"position":{"line":1,"character":1}}),
    );
    assert_eq!(definition["result"]["uri"], dep);
}

#[test]
fn outlines_and_declaration_hover_survive_broken_code_imports() {
    let temp = tempfile::tempdir().unwrap();
    let uri = url::Url::from_file_path(temp.path().join("lib.notc"))
        .unwrap()
        .to_string();
    let mut server = Lsp::initialized(temp.path());
    let source = "import \"missing.notc\": *;\nfn kept() -> InlineContent { [body] }";
    server.open(&uri, 1, source);
    let outline = server.request(
        2,
        "textDocument/documentSymbol",
        json!({"textDocument":{"uri":uri}}),
    );
    assert_eq!(outline["result"][0]["name"], "kept");
    let hover = server.request(
        3,
        "textDocument/hover",
        json!({"textDocument":{"uri":uri},"position":{"line":1,"character":4}}),
    );
    assert_eq!(
        hover["result"]["contents"]["value"],
        "fn kept() -> InlineContent { [body] }"
    );
}
