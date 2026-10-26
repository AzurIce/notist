use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_notist"));
    command.current_dir(root);
    command
}
fn write(root: &Path, path: &str, source: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, source).unwrap();
}

#[test]
fn build_replaces_owned_sites_prunes_stale_pages_and_preserves_output_on_errors() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", "[book]\ntitle='Book'\noutput='public'");
    write(root, "README.not", "= Home\n\n[Guide](guide/README.md)");
    write(root, "guide/README.md", "# Guide\n\nContent");
    let result = command(root).args(["book", "build"]).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let page = root.join("public/index.html");
    let original = std::fs::read(&page).unwrap();
    assert!(root.join("public/guide/index.html").is_file());
    write(root, "README.not", "= Home\n\n![Missing](bad.png)");
    let result = command(root).args(["book", "build"]).output().unwrap();
    assert!(!result.status.success());
    assert_eq!(std::fs::read(&page).unwrap(), original);
    write(root, "README.not", "= Updated");
    std::fs::remove_dir_all(root.join("guide")).unwrap();
    let result = command(root).args(["book", "build"]).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!root.join("public/guide").exists());
    assert!(std::fs::read_to_string(page).unwrap().contains("Updated"));
    write(root, "unowned/keep.txt", "Preserve me");
    let result = command(root)
        .args(["book", "build", "--out-dir", "unowned"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(
        std::fs::read_to_string(root.join("unowned/keep.txt")).unwrap(),
        "Preserve me"
    );
    let result = command(root)
        .args(["book", "build", "--out-dir", "."])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(root.join("README.not").is_file());
}

#[test]
fn book_build_requires_output_and_resolves_cli_output_from_working_directory() {
    for config in [None, Some(""), Some("[book]\ntitle='Handbook'")] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, "vault/README.md", "# Home");
        if let Some(config) = config {
            write(root, "vault/Notist.toml", config);
        }
        let result = command(root)
            .args(["book", "build", "vault"])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("set [book].output or pass --out-dir")
        );
        assert!(!root.join("vault/target").exists());
        assert!(!root.join("vault/book").exists());

        let result = command(root)
            .args(["book", "build", "vault", "--out-dir", "published"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(root.join("published/index.html").is_file());
        assert!(!root.join("vault/published").exists());
    }
}

#[test]
fn book_build_publishes_mounted_assets_and_prunes_unreferenced_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "docs/Notist.toml",
        "[book]\noutput='../public'\n[[book.mounts]]\nsource='../art'\npath='art'",
    );
    write(
        root,
        "docs/README.md",
        "# Home\n\n[Drill](art/drill/README.md)",
    );
    write(
        root,
        "art/drill/README.md",
        "# Drill\n\n![Preview](renders/hero.png)\n\n[Download](model.glb)",
    );
    write(root, "art/drill/renders/hero.png", "image bytes");
    write(root, "art/drill/model.glb", "model bytes");
    write(root, "art/drill/private.blend", "unused");
    let result = command(root)
        .args(["book", "build", "docs"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(root.join("public/art/drill/index.html").is_file());
    assert_eq!(
        std::fs::read(root.join("public/art/drill/model.glb")).unwrap(),
        b"model bytes"
    );
    assert!(!root.join("public/art/drill/private.blend").exists());
    write(root, "art/drill/README.md", "# Drill updated");
    let result = command(root)
        .args(["book", "build", "docs"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!root.join("public/art/drill/model.glb").exists());
    assert!(!root.join("public/art/drill/renders").exists());
    assert_eq!(
        std::fs::read(root.join("art/drill/model.glb")).unwrap(),
        b"model bytes"
    );
}

#[cfg(unix)]
#[test]
fn book_mounts_reject_symlink_sources_and_resources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "docs/Notist.toml",
        "[book]\noutput='../public'\n[[book.mounts]]\nsource='../art'\npath='art'",
    );
    write(root, "real/README.md", "# Art");
    std::os::unix::fs::symlink(root.join("real"), root.join("art")).unwrap();
    let result = command(root)
        .args(["book", "build", "docs"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("regular files or directories"));
    std::fs::remove_file(root.join("art")).unwrap();
    write(root, "art/README.md", "# Art\n\n![Image](image.png)");
    write(root, "outside.png", "image");
    std::os::unix::fs::symlink(root.join("outside.png"), root.join("art/image.png")).unwrap();
    let result = command(root)
        .args(["book", "build", "docs"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("regular files or directories"));
}

#[cfg(unix)]
#[test]
fn html_and_site_publication_reject_symlink_component_resources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", "");
    write(root, "lib.notc", "import \"widgets/lib.notc\" as widgets;");
    write(
        root,
        "README.not",
        "#import \"widgets/lib.notc\" as widgets;\n#widgets.badge(\"Hello\")",
    );
    write(root, "widgets/Notist.toml", "");
    write(
        root,
        "widgets/lib.notc",
        "fn badge(label: String) -> InlineContent;",
    );
    write(
        root,
        "component.js",
        "export default class extends HTMLElement {}",
    );
    std::fs::create_dir_all(root.join("widgets/components")).unwrap();
    std::os::unix::fs::symlink(root.join("component.js"), root.join("widgets/badge.js")).unwrap();
    for args in [
        vec!["html", "README.not", "--out-dir", "published"],
        vec!["book", "build", "--out-dir", "published"],
    ] {
        let result = command(root).args(args).output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("regular files or directories"));
        assert!(!root.join("published").exists());
    }
}

#[test]
fn html_export_is_independent_of_book_structure_and_theme() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[book]\ntheme='missing-theme'\nadditional-css=['missing.css']",
    );
    write(root, "README.not", "= Standalone\n\nBody");
    write(root, "SUMMARY.not", "Not a valid book structure.");
    let exported = command(root)
        .args(["html", "README.not", "--out-dir", "document"])
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let page = std::fs::read_to_string(root.join("document/index.html")).unwrap();
    assert!(page.contains("<h1>Standalone</h1>"));
    assert!(!page.contains("book-nav"));
    assert!(root.join("document/_notist/document.css").is_file());
    assert!(!root.join("document/_notist/theme").exists());
    let built = command(root).args(["book", "build"]).output().unwrap();
    assert!(!built.status.success());
}

struct Server {
    child: Child,
    address: String,
    log: tempfile::NamedTempFile,
}
impl Server {
    fn start(root: &Path) -> Self {
        Self::with_options(root, &[])
    }
    fn with_options(root: &Path, options: &[&str]) -> Self {
        let log = tempfile::NamedTempFile::new().unwrap();
        let mut child = command(root)
            .args(["book", "serve", "--address", "127.0.0.1:0"])
            .args(options)
            .stdout(Stdio::piped())
            .stderr(log.reopen().unwrap())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            BufReader::new(stdout).read_line(&mut line).unwrap();
            let _ = sender.send(line);
        });
        let line = match receiver.recv_timeout(Duration::from_secs(15)) {
            Ok(line) => line,
            Err(error) => {
                let _ = child.kill();
                panic!("serve did not start: {error}");
            }
        };
        let address = line
            .trim()
            .strip_prefix("serve http://")
            .unwrap()
            .trim_end_matches('/')
            .into();
        Self {
            child,
            address,
            log,
        }
    }
    fn wait_for_log(&self, message: &str) -> String {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let log = std::fs::read_to_string(self.log.path()).unwrap();
            if log.contains(message) {
                return log;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "missing {message:?} in {log}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn connect(&self, path: &str) -> TcpStream {
        let mut stream = TcpStream::connect(&self.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.address
        )
        .unwrap();
        stream
    }
    fn get(&self, path: &str) -> String {
        let mut text = String::new();
        self.connect(path).read_to_string(&mut text).unwrap();
        text
    }
    fn events(&self) -> BufReader<TcpStream> {
        let mut reader = BufReader::new(self.connect("/_notist/serve/events"));
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line == "\r\n" {
                break;
            }
        }
        reader
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn update(reader: &mut BufReader<TcpStream>, predicate: impl Fn(&Value) -> bool) -> Value {
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert!(!line.is_empty(), "SSE disconnected");
        if let Some(data) = line.strip_prefix("data: ") {
            let event: Value = serde_json::from_str(data.trim()).unwrap();
            if predicate(&event) {
                return event;
            }
        }
    }
}

#[test]
fn serve_watches_mounted_pages_resources_and_new_subdirectories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "docs/Notist.toml",
        "[book]\noutput='../public'\n[[book.mounts]]\nsource='../art'\npath='art'",
    );
    write(root, "docs/README.md", "# Home");
    write(root, "art/README.md", "# Art\n\n![Image](image.png)");
    write(root, "art/image.png", "before");
    let server = Server::start(&root.join("docs"));
    let mut events = server.events();
    let baseline = update(&mut events, |_| true);
    assert!(server.get("/art/image.png").ends_with("before"));
    write(root, "art/image.png", "after");
    let changed = update(&mut events, |event| {
        event["revision"].as_u64() > baseline["revision"].as_u64()
    });
    assert!(changed["report"]["errors"].as_array().unwrap().is_empty());
    assert!(server.get("/art/image.png").ends_with("after"));
    write(root, "art/new/deep/README.md", "# New mounted chapter");
    let added = update(&mut events, |event| {
        event["revision"].as_u64() > changed["revision"].as_u64()
    });
    assert!(added["report"]["errors"].as_array().unwrap().is_empty());
    assert!(server.get("/art/new/deep/").contains("New mounted chapter"));
    write(root, "art/new/deep/README.md", "# Updated mounted chapter");
    update(&mut events, |event| {
        event["revision"].as_u64() > added["revision"].as_u64()
    });
    assert!(
        server
            .get("/art/new/deep/")
            .contains("Updated mounted chapter")
    );
}

#[test]
fn serve_exposes_clean_urls_reload_events_and_last_successful_build() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", "[book]\noutput='public'");
    write(root, "README.md", "# Home");
    write(root, "guide/README.not", "= Guide");
    let server = Server::start(root);
    let page = server.get("/guide/");
    assert!(page.starts_with("HTTP/1.1 200"));
    assert!(page.contains("../_notist/serve/client.js?v=1"));
    assert!(
        server
            .get("/_notist/serve/client.js")
            .contains("new EventSource")
    );
    assert!(server.get("/guide").starts_with("HTTP/1.1 308"));
    assert!(server.get("/missing/").starts_with("HTTP/1.1 404"));
    assert!(server.get("/%2e%2e/private").starts_with("HTTP/1.1 400"));
    assert!(
        server
            .get("/_notist/theme/book.css")
            .contains("Content-Type: text/css")
    );
    let mut events = server.events();
    let baseline = update(&mut events, |_| true);
    write(root, "README.md", "# Updated");
    let changed = update(&mut events, |event| {
        event["revision"].as_u64() > baseline["revision"].as_u64()
    });
    assert!(changed["report"]["errors"].as_array().unwrap().is_empty());
    assert!(server.get("/").contains("Updated"));
    write(root, "README.md", "# Broken\n\n![Missing](missing.png)");
    let failed = update(&mut events, |event| event["report"]["status"] == "failed");
    assert_eq!(failed["revision"], changed["revision"]);
    assert!(server.get("/").contains("Updated"));
    write(root, "new/README.not", "= New page");
    write(root, "README.md", "# Recovered\n\n[New](new/README.not)");
    update(&mut events, |event| {
        event["revision"].as_u64() > changed["revision"].as_u64()
    });
    assert!(server.get("/new/").contains("New page"));
    assert!(server.get("/").contains("href=\"new/\""));
    assert!(
        !root.join("public").exists(),
        "the server serves memory artifacts"
    );
}

#[test]
fn serve_requires_output_and_recovers_when_it_is_configured() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "README.md", "# Home");
    let server = Server::start(root);
    assert!(server.get("/").starts_with("HTTP/1.1 503"));
    let mut events = server.events();
    let failed = update(&mut events, |event| event["report"]["status"] == "failed");
    assert_eq!(failed["revision"], 0);
    assert!(
        failed["report"]["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("book output")
    );
    write(root, "Notist.toml", "[book]\noutput='published'");
    update(&mut events, |event| event["revision"].as_u64().unwrap() > 0);
    assert!(server.get("/").contains("Home"));
    assert!(!root.join("published").exists());
}

#[test]
fn serve_replaces_diagnostics_and_distinguishes_recovered_errors_from_warnings() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", "[book]\noutput='public'");
    write(
        root,
        "README.not",
        "#import \"lib.notc\" as demo;\n#demo.content() #demo.other()",
    );
    write(
        root,
        "lib.notc",
        "fn content() -> InlineContent { [before] } fn other() -> InlineContent { [other] }",
    );
    let server = Server::start(root);
    let mut events = server.events();
    let ready = update(&mut events, |_| true);
    assert_eq!(ready["report"]["status"], "ready");

    write(
        root,
        "lib.notc",
        "fn content() -> InlineContent { compile_error(\"first error\") } fn other() -> InlineContent { compile_error(\"second error\") }",
    );
    let first = update(&mut events, |event| event["report"]["status"] == "errors");
    assert!(first["revision"].as_u64() > ready["revision"].as_u64());
    assert!(
        first["summary"]
            .as_str()
            .unwrap()
            .contains("Preview updated with")
    );
    let detail = first["report"]["errors"][0]["detail"].as_str().unwrap();
    assert!(detail.contains("lib.notc:1:"), "{detail}");
    assert!(detail.contains("compile_error"), "{detail}");
    assert!(
        !detail.contains('\x1b'),
        "browser code frames must be plain text"
    );
    let initial_errors = first["report"]["errors"].as_array().unwrap().len();
    assert!(initial_errors >= 2);
    assert_eq!(first["report"]["resolved"], false);
    assert!(server.get("/").contains("first error"));

    write(
        root,
        "lib.notc",
        "fn content() -> InlineContent { [partly fixed] } fn other() -> InlineContent { compile_error(\"second error\") }",
    );
    let second = update(&mut events, |event| {
        event["report"]["status"] == "errors"
            && event["report"]["errors"]
                .to_string()
                .contains("second error")
    });
    assert!(
        !second["report"]["errors"]
            .to_string()
            .contains("first error")
    );
    let remaining_errors = second["report"]["errors"].as_array().unwrap().len();
    assert!(remaining_errors > 0 && remaining_errors < initial_errors);
    assert_eq!(second["report"]["resolved"], false);
    let latest = update(&mut server.events(), |_| true);
    assert_eq!(
        latest["report"], second["report"],
        "new connections receive current diagnostics"
    );

    // A partial fix and a warning must not imply that all errors are resolved.
    write(
        root,
        "README.not",
        "#import \"lib.notc\" as demo;\n#demo.content() #demo.other()\n\n[Missing](missing.not)",
    );
    update(&mut events, |event| {
        event["report"]["status"] == "errors"
            && !event["report"]["warnings"].as_array().unwrap().is_empty()
    });
    write(
        root,
        "lib.notc",
        "fn content() -> InlineContent { [fixed] } fn other() -> InlineContent { [other] }",
    );
    let warning = update(&mut events, |event| event["report"]["status"] == "warnings");
    assert_eq!(warning["report"]["resolved"], true);
    assert!(warning["report"]["errors"].as_array().unwrap().is_empty());
    assert!(!warning["report"]["warnings"].as_array().unwrap().is_empty());
    assert!(
        warning["report"]["warnings"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("README.not:")
    );
    assert!(server.get("/").contains("fixed"));

    write(
        root,
        "README.not",
        "#import \"lib.notc\" as demo;\n#demo.content() #demo.other()",
    );
    let clean = update(&mut events, |event| event["report"]["status"] == "ready");
    assert!(clean["report"]["errors"].as_array().unwrap().is_empty());
    assert!(clean["report"]["warnings"].as_array().unwrap().is_empty());
    assert!(clean["report"]["build"].as_u64() > second["report"]["build"].as_u64());
    let log = server.wait_for_log("All errors resolved");
    assert!(log.contains("Preview updated with"));
    assert!(log.contains("[notist #"));
    assert!(
        !log.contains('\x1b'),
        "redirected output must not contain terminal escapes"
    );
}

#[test]
fn serve_no_clear_screen_keeps_failed_and_resolved_build_output() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", "[book]\noutput='public'");
    write(root, "README.md", "# Home");
    let server = Server::with_options(root, &["--no-clear-screen"]);
    let mut events = server.events();
    let ready = update(&mut events, |_| true);
    write(root, "README.md", "# Broken\n\n![Missing](missing.png)");
    let failed = update(&mut events, |event| event["report"]["status"] == "failed");
    assert_eq!(failed["revision"], ready["revision"]);
    assert!(
        failed["summary"]
            .as_str()
            .unwrap()
            .contains("showing previous preview")
    );
    assert!(server.get("/").contains("Home"));
    write(root, "README.md", "# Fixed");
    let fixed = update(&mut events, |event| event["report"]["status"] == "ready");
    assert_eq!(fixed["report"]["resolved"], true);
    assert!(fixed["report"]["errors"].as_array().unwrap().is_empty());
    let log = server.wait_for_log("All errors resolved");
    assert!(log.contains("Build failed"));
    assert!(log.contains("Updated"));
    assert!(!log.contains('\x1b'));
}

#[test]
fn summary_limits_build_output_and_served_pages() {
    for summary in ["SUMMARY.not", "SUMMARY.md"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        write(root, "Notist.toml", "[book]\noutput='public'");
        write(root, "README.not", "= Home");
        write(root, "README.md", "# Unlisted route collision");
        write(root, "ai/broken.not", "#missing[]");
        write(root, "_logs/entry.md", "# Log entry");
        write(root, "later.md", "# Later");
        write(root, summary, "- [Home](README.not)\n- [Logs](_logs/)");
        let result = command(root).args(["book", "build"]).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(root.join("public/index.html").is_file());
        assert!(root.join("public/_logs/entry/index.html").is_file());
        assert!(!root.join("public/ai").exists());

        let server = Server::start(root);
        assert!(server.get("/").starts_with("HTTP/1.1 200"));
        assert!(server.get("/_logs/entry/").contains("Log entry"));
        assert!(server.get("/ai/broken/").starts_with("HTTP/1.1 404"));
        assert!(server.get("/later/").starts_with("HTTP/1.1 404"));
        let mut events = server.events();
        let baseline = update(&mut events, |_| true);
        write(
            root,
            summary,
            "- [Home](README.not)\n- [Logs](_logs/)\n- [Later](later.md)",
        );
        let changed = update(&mut events, |event| {
            event["revision"].as_u64() > baseline["revision"].as_u64()
                || event["report"]["status"] == "failed"
        });
        assert!(
            changed["report"]["errors"].as_array().unwrap().is_empty(),
            "{changed}"
        );
        let page = server.get("/later/");
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains(">Later</h1>"), "{page}");
    }
}

#[test]
fn broken_hyperlinks_warn_without_failing_build_or_live_reload() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "Notist.toml", "[book]\noutput='public'");
    write(
        root,
        "README.md",
        "# Home\n\n[Missing](missing.md?view=full#section)",
    );
    let result = command(root).args(["book", "build"]).output().unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");
    assert!(stderr.contains("warning[semantic]"), "{stderr}");
    assert!(!stderr.contains("error["), "{stderr}");
    assert!(
        std::fs::read_to_string(root.join("public/index.html"))
            .unwrap()
            .contains("href=\"missing/?view=full#section\"")
    );
    assert!(!root.join("public/missing").exists());

    let server = Server::start(root);
    assert!(server.get("/").starts_with("HTTP/1.1 200"));
    assert!(server.get("/missing/").starts_with("HTTP/1.1 404"));
    let mut events = server.events();
    let baseline = update(&mut events, |_| true);
    assert!(baseline["report"]["errors"].as_array().unwrap().is_empty());
    write(root, "README.md", "# Updated\n\n[Missing](another.md)");
    let changed = update(&mut events, |event| {
        event["revision"].as_u64() > baseline["revision"].as_u64()
            || event["report"]["status"] == "failed"
    });
    assert!(
        changed["revision"].as_u64() > baseline["revision"].as_u64(),
        "{changed}"
    );
    assert!(
        changed["report"]["errors"].as_array().unwrap().is_empty(),
        "{changed}"
    );
    assert!(server.get("/").contains("Updated"));
    assert!(server.get("/another/").starts_with("HTTP/1.1 404"));
}

#[test]
fn outside_hyperlinks_warn_and_stay_out_of_build_and_serve() {
    let temp = tempfile::tempdir().unwrap();
    let parent = temp.path();
    let root = parent.join("docs");
    write(&root, "Notist.toml", "[book]\noutput='public'");
    write(
        &root,
        "README.md",
        "# Home\n\n[Outside](../private/secret.md?view=full#section)",
    );
    write(
        parent,
        "private/secret.md",
        "# Private content must stay outside the book",
    );
    let result = command(&root).args(["book", "build"]).output().unwrap();
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(result.status.success(), "{stderr}");
    assert!(stderr.contains("warning[semantic]"), "{stderr}");
    assert!(stderr.contains("outside Vault"), "{stderr}");
    assert!(!stderr.contains("error["), "{stderr}");
    let output = std::fs::read_to_string(root.join("public/index.html")).unwrap();
    assert!(output.contains("href=\"../private/secret.md?view=full#section\""));
    assert!(!output.contains("Private content"));
    assert!(!root.join("public/private").exists());

    let server = Server::start(&root);
    assert!(server.get("/").starts_with("HTTP/1.1 200"));
    assert!(server.get("/private/secret.md").starts_with("HTTP/1.1 404"));
    let mut events = server.events();
    let baseline = update(&mut events, |_| true);
    assert!(baseline["report"]["errors"].as_array().unwrap().is_empty());
    write(
        &root,
        "README.md",
        "# Updated\n\n[Outside](../private/missing.md)",
    );
    let changed = update(&mut events, |event| {
        event["revision"].as_u64() > baseline["revision"].as_u64()
            || event["report"]["status"] == "failed"
    });
    assert!(
        changed["report"]["errors"].as_array().unwrap().is_empty(),
        "{changed}"
    );
    assert!(changed["revision"].as_u64() > baseline["revision"].as_u64());
    assert!(server.get("/").contains("Updated"));
}

#[test]
fn serve_recovers_from_initial_template_errors_and_watches_external_modules() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    write(
        &root,
        "Notist.toml",
        "[book]\noutput='public'\ntheme='theme'",
    );
    write(
        &root,
        "lib.notc",
        "import \"../widgets/lib.notc\" as widgets;",
    );
    write(
        &root,
        "README.not",
        "#import \"../widgets/lib.notc\" as widgets;\n= Home\n\n#widgets.badge(\"Hello\")",
    );
    write(&root, "theme/index.hbs", "{{page.missing}}");
    write(temp.path(), "widgets/Notist.toml", "");
    write(
        temp.path(),
        "widgets/lib.notc",
        "fn badge(label: String) -> InlineContent;",
    );
    write(
        temp.path(),
        "widgets/badge.js",
        "export default class extends HTMLElement { /* before */ }",
    );
    let server = Server::start(&root);
    assert!(server.get("/").starts_with("HTTP/1.1 503"));
    let mut events = server.events();
    update(&mut events, |event| event["report"]["status"] == "failed");
    write(&root, "theme/index.hbs", "{{page.title}} {{{page.html}}}");
    let good = update(&mut events, |event| event["revision"].as_u64().unwrap() > 0);
    let path = format!(
        "/_notist/modules/{}/badge.js",
        notist_html::components::module_key(temp.path().join("widgets/lib.notc").to_str().unwrap())
    );
    assert!(server.get(&path).contains("before"));
    write(
        temp.path(),
        "widgets/badge.js",
        "export default class extends HTMLElement { /* after */ }",
    );
    update(&mut events, |event| {
        event["revision"].as_u64() > good["revision"].as_u64()
    });
    assert!(server.get(&path).contains("after"));
}
