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
    write(root, "Notist.toml", "[site]\ntitle='Book'\noutput='public'");
    write(root, "README.not", "= Home\n\n[Guide](guide/README.md)");
    write(root, "guide/README.md", "# Guide\n\nContent");
    let result = command(root).args(["build"]).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let page = root.join("public/index.html");
    let original = std::fs::read(&page).unwrap();
    assert!(root.join("public/guide/index.html").is_file());
    write(root, "README.not", "= Home\n\n[Missing](bad.md)");
    let result = command(root).args(["build"]).output().unwrap();
    assert!(!result.status.success());
    assert_eq!(std::fs::read(&page).unwrap(), original);
    write(root, "README.not", "= Updated");
    std::fs::remove_dir_all(root.join("guide")).unwrap();
    let result = command(root).args(["build"]).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!root.join("public/guide").exists());
    assert!(std::fs::read_to_string(page).unwrap().contains("Updated"));
    write(root, "unowned/keep.txt", "Preserve me");
    let result = command(root)
        .args(["build", "--out-dir", "unowned"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(
        std::fs::read_to_string(root.join("unowned/keep.txt")).unwrap(),
        "Preserve me"
    );
    let result = command(root)
        .args(["build", "--out-dir", "."])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(root.join("README.not").is_file());
}

#[cfg(unix)]
#[test]
fn html_and_site_publication_reject_symlink_component_resources() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[dependencies]\nwidgets={path='widgets'}",
    );
    write(root, "README.not", "#widgets::badge(\"Hello\")");
    write(root, "widgets/Notist.toml", "[package]\nname='widgets'");
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
    std::os::unix::fs::symlink(
        root.join("component.js"),
        root.join("widgets/components/badge.js"),
    )
    .unwrap();
    for args in [
        vec!["html", "README.not", "--out-dir", "published"],
        vec!["build", "--out-dir", "published"],
    ] {
        let result = command(root).args(args).output().unwrap();
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("regular files or directories"));
        assert!(!root.join("published").exists());
    }
}

struct Preview {
    child: Child,
    address: String,
}
impl Preview {
    fn start(root: &Path) -> Self {
        let mut child = command(root)
            .args(["preview", "--address", "127.0.0.1:0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
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
                panic!("preview did not start: {error}");
            }
        };
        let address = line
            .trim()
            .strip_prefix("preview http://")
            .unwrap()
            .trim_end_matches('/')
            .into();
        Self { child, address }
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
        let mut reader = BufReader::new(self.connect("/_notist/preview/events"));
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
impl Drop for Preview {
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
fn preview_serves_clean_urls_reload_events_and_last_successful_build() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "README.md", "# Home");
    write(root, "guide/README.not", "= Guide");
    let preview = Preview::start(root);
    let page = preview.get("/guide/");
    assert!(page.starts_with("HTTP/1.1 200"));
    assert!(page.contains("../_notist/preview/client.js?v=1"));
    assert!(preview.get("/guide").starts_with("HTTP/1.1 308"));
    assert!(preview.get("/missing/").starts_with("HTTP/1.1 404"));
    assert!(preview.get("/%2e%2e/private").starts_with("HTTP/1.1 400"));
    assert!(
        preview
            .get("/_notist/theme/site.css")
            .contains("Content-Type: text/css")
    );
    let mut events = preview.events();
    let baseline = update(&mut events, |_| true);
    write(root, "README.md", "# Updated");
    let changed = update(&mut events, |event| {
        event["revision"].as_u64() > baseline["revision"].as_u64()
    });
    assert!(changed["error"].is_null());
    assert!(preview.get("/").contains("Updated"));
    write(root, "README.md", "# Broken\n\n[Missing](missing.md)");
    let failed = update(&mut events, |event| event["error"].is_string());
    assert_eq!(failed["revision"], changed["revision"]);
    assert!(preview.get("/").contains("Updated"));
    write(root, "new/README.not", "= New page");
    write(root, "README.md", "# Recovered\n\n[New](new/README.not)");
    update(&mut events, |event| {
        event["revision"].as_u64() > changed["revision"].as_u64()
    });
    assert!(preview.get("/new/").contains("New page"));
    assert!(preview.get("/").contains("href=\"new/\""));
    assert!(
        !root.join("target/site").exists(),
        "preview serves memory artifacts"
    );
}

#[test]
fn preview_recovers_from_initial_template_errors_and_watches_external_packages() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    write(
        &root,
        "Notist.toml",
        "[dependencies]\nwidgets={path='../widgets'}\n[site]\ntheme='theme'",
    );
    write(&root, "README.not", "= Home\n\n#widgets::badge(\"Hello\")");
    write(&root, "theme/index.hbs", "{{page.missing}}");
    write(
        temp.path(),
        "widgets/Notist.toml",
        "[package]\nname='widgets'",
    );
    write(
        temp.path(),
        "widgets/lib.notc",
        "fn badge(label: String) -> InlineContent;",
    );
    write(
        temp.path(),
        "widgets/components/badge.js",
        "export default class extends HTMLElement { /* before */ }",
    );
    let preview = Preview::start(&root);
    assert!(preview.get("/").starts_with("HTTP/1.1 503"));
    let mut events = preview.events();
    update(&mut events, |event| event["error"].is_string());
    write(&root, "theme/index.hbs", "{{page.title}} {{{page.html}}}");
    let good = update(&mut events, |event| event["revision"].as_u64().unwrap() > 0);
    let path = "/_notist/packages/widgets/components/badge.js";
    assert!(preview.get(path).contains("before"));
    write(
        temp.path(),
        "widgets/components/badge.js",
        "export default class extends HTMLElement { /* after */ }",
    );
    update(&mut events, |event| {
        event["revision"].as_u64() > good["revision"].as_u64()
    });
    assert!(preview.get(path).contains("after"));
}
