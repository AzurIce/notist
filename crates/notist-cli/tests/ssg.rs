use std::{
    io::Read,
    net::TcpStream,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}
fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_notist"));
    command.current_dir(root);
    command
}
fn request(address: &str, path: &str) -> String {
    use std::io::Write;
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut result = String::new();
    stream.read_to_string(&mut result).unwrap();
    result
}

#[test]
#[cfg(unix)]
fn site_commands_edit_inputs_in_order_and_preserve_publication_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(root, "content/index.not", "= Original");
    write(root, "edit.py", r#"
import json, sys
from pathlib import Path
for line in sys.stdin:
    r = json.loads(line)
    if r['event'] == 'before_compile':
        Path(sys.argv[1]).write_text(line)
        response = json.loads(Path(sys.argv[2]).read_text())
    else:
        Path(sys.argv[1] + '.template').write_text(line)
        data = r['data']
        data['previous'] = sys.argv[1]
        response = dict(version=2, event=r['event'], data=data)
    print(json.dumps(response), flush=True)
"#);
    write(
        root,
        "Notist.toml",
        "[package]\nname='test'\n[site]\noutput='public'\n[site.extensions.first]\ncommand=['python3','edit.py','first-request.json','first-response.json']\n[site.extensions.second]\ncommand=['python3','edit.py','second-request.json','second-response.json']",
    );
    write(
        root,
        "first-response.json",
        r##"{"version":2,"event":"before_compile","documents":{"index.not":"= First"}}"##,
    );
    write(
        root,
        "second-response.json",
        r##"{"version":2,"event":"before_compile","documents":{"index.not":"= Final\n\n[New](new/page.md)","new/page.md":"# New"}}"##,
    );
    let result = command(root).args(["build"]).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let request: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("second-request.json")).unwrap())
            .unwrap();
    assert_eq!(request["documents"]["index.not"], "= First");
    assert!(request.get("data").is_none());
    let request: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("second-request.json.template")).unwrap()
    ).unwrap();
    assert_eq!(request["data"]["previous"], "first-request.json");
    assert!(request["pages"].as_array().unwrap().iter().all(|p| !p["html"].as_str().unwrap().is_empty()));
    let original = std::fs::read_to_string(root.join("public/index.html")).unwrap();
    assert!(original.contains("Final"));
    assert!(original.contains("new/page.html"));
    assert!(root.join("public/new/page.html").is_file());
    assert!(!root.join("content/new/page.md").exists());
    write(
        root,
        "second-response.json",
        r##"{"version":2,"event":"before_compile","errors":["missing manifest"]}"##,
    );
    let result = command(root).args(["build"]).output().unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing manifest"));
    assert_eq!(
        std::fs::read_to_string(root.join("public/index.html")).unwrap(),
        original
    );
    write(
        root,
        "second-response.json",
        r##"{"version":2,"event":"before_compile","documents":{"index.not":"= Latest"}}"##,
    );
    assert!(
        command(root)
            .args(["build"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!root.join("public/new/page.html").exists());
}

#[test]
#[cfg(unix)]
fn preview_watches_external_extension_inputs_recursively_even_after_failure() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("site");
    std::fs::create_dir_all(temp.path().join("external")).unwrap();
    write(&root, "content/index.not", "= Original");
    write(
        &root,
        "edit.py",
        r#"
import json, sys
from pathlib import Path
for line in sys.stdin:
    r = json.loads(line)
    response = dict(version=2, event=r['event'])
    try:
        if r['event'] == 'before_compile':
            response = json.loads(Path('../external/nested/response.json').read_text())
    except Exception as error:
        response['errors'] = [str(error)]
    print(json.dumps(response), flush=True)
"#,
    );
    write(
        &root,
        "Notist.toml",
        "[package]\nname='test'\n[site]\noutput='public'\n[site.extensions.edit]\ncommand=['python3','edit.py']\nwatch=['../external']",
    );
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let address = format!("127.0.0.1:{port}");
    let mut child = command(&root)
        .args(["serve", "--address", &address, "--no-clear-screen"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let result = std::panic::catch_unwind(|| {
        let wait = |needle: &str| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if TcpStream::connect(&address).is_ok() {
                    let response = request(&address, "/");
                    if response.contains(needle) {
                        return response;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "preview did not produce {needle}"
                );
                std::thread::sleep(Duration::from_millis(30));
            }
        };
        wait("HTTP/1.1 503");
        // The directory and its input did not exist at the failed first build.
        write(
            temp.path(),
            "external/nested/response.json",
            r##"{"version":2,"event":"before_compile","documents":{"index.not":"= Generated"}}"##,
        );
        wait("<h1 id=\"generated\">Generated");
        write(
            temp.path(),
            "external/nested/response.json",
            r##"{"version":2,"event":"before_compile","errors":["temporary failure"]}"##,
        );
        std::thread::sleep(Duration::from_millis(500));
        assert!(request(&address, "/").contains("Generated"));
        write(
            temp.path(),
            "external/nested/response.json",
            r##"{"version":2,"event":"before_compile","documents":{"index.not":"= Recovered"}}"##,
        );
        wait("Recovered");
        std::fs::remove_dir_all(temp.path().join("external")).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        write(
            temp.path(),
            "external/nested/response.json",
            r##"{"version":2,"event":"before_compile","documents":{"index.not":"= Restored"}}"##,
        );
        wait("Restored");
        write(
            temp.path(),
            "external/nested/response.json",
            r##"{"version":2,"event":"before_compile","documents":{"index.not":"= Watched again"}}"##,
        );
        wait("Watched again");
        assert!(!root.join("public").exists());
    });
    let _ = child.kill();
    let _ = child.wait();
    result.unwrap();
}

#[test]
fn preview_watches_concrete_hidden_data_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[package]\nname='hidden'\n[site]\noutput='public'",
    );
    write(
        root,
        "content/index.not",
        "#import \"../lib.notc\" as data;\n#data.view()\n",
    );
    write(
        root,
        "lib.notc",
        "fn view() -> InlineContent { [#text(text: read_text(p\"content/.data/value.txt\"))] }",
    );
    write(root, "content/.data/value.txt", "original hidden value");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let address = format!("127.0.0.1:{port}");
    let mut child = command(root)
        .args(["serve", "--address", &address, "--no-clear-screen"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let result = std::panic::catch_unwind(|| {
        let wait = |needle: &str| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if TcpStream::connect(&address).is_ok() && request(&address, "/").contains(needle) {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "hidden dependency did not update to {needle}"
                );
                std::thread::sleep(Duration::from_millis(30));
            }
        };
        wait("original hidden value");
        write(root, "content/.data/value.txt", "updated hidden value");
        wait("updated hidden value");
        std::fs::remove_file(root.join("content/.data/value.txt")).unwrap();
        std::thread::sleep(Duration::from_millis(500));
        write(root, "content/.data/value.txt", "restored hidden value");
        wait("restored hidden value");
    });
    let _ = child.kill();
    let _ = child.wait();
    result.unwrap();
}

#[test]
fn site_publication_prunes_stale_files_and_keeps_last_success_on_failure() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[package]\nname='test'\n[site]\noutput='public'",
    );
    write(root, "content/index.not", "= Home");
    write(root, "content/sub/page.md", "# Page");
    write(root, "static/extra.txt", "asset");
    let result = command(root).args(["build"]).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(root.join("public/sub/page.html").is_file());
    assert!(!root.join("public/sub/index.html").exists());
    let original = std::fs::read(root.join("public/index.html")).unwrap();
    write(
        root,
        "content/index.not",
        "= Home\n\n![Missing](missing.png)",
    );
    assert!(
        !command(root)
            .args(["build"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        std::fs::read(root.join("public/index.html")).unwrap(),
        original
    );
    write(root, "content/index.not", "= Updated");
    std::fs::remove_file(root.join("content/sub/page.md")).unwrap();
    assert!(
        command(root)
            .args(["build"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(!root.join("public/sub/page.html").exists());
    write(root, "unowned/keep.txt", "keep");
    assert!(
        !command(root)
            .args(["build", "--out-dir", "unowned"])
            .output()
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        std::fs::read_to_string(root.join("unowned/keep.txt")).unwrap(),
        "keep"
    );
}

#[test]
fn site_preview_uses_local_urls_watches_templates_and_retains_failed_build() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    write(
        root,
        "Notist.toml",
        "[package]\nname='test'\n[site]\nbase_url='https://example.com/project/'",
    );
    write(root, "content/index.not", "= Home");
    write(
        root,
        "templates/page.tera",
        "<html><body>{{url(path='style.css')}} {{page.html | safe}}</body></html>",
    );
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let address = format!("127.0.0.1:{port}");
    let mut child = command(root)
        .args(["serve", "--address", &address, "--no-clear-screen"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let result = std::panic::catch_unwind(|| {
        let wait = |needle: &str| {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                if TcpStream::connect(&address).is_ok() {
                    let response = request(&address, "/");
                    if response.contains(needle) {
                        return response;
                    }
                }
                assert!(
                    Instant::now() < deadline,
                    "preview did not produce {needle}"
                );
                std::thread::sleep(Duration::from_millis(30));
            }
        };
        let home = wait("<h1 id=\"home\">Home");
        assert!(home.contains("/style.css"));
        assert!(!home.contains("https://example.com"));
        assert!(home.contains("_notist/serve/client.js"));
        write(
            root,
            "templates/page.tera",
            "<html><body>NEW {{page.html | safe}}</body></html>",
        );
        wait("NEW");
        write(
            root,
            "content/index.not",
            "= Home\n\n![Missing](missing.png)",
        );
        std::thread::sleep(Duration::from_millis(500));
        assert!(request(&address, "/").contains("NEW"));
        write(root, "content/index.not", "= Recovered");
        wait("Recovered");
        assert!(request(&address, "/unknown/").starts_with("HTTP/1.1 404"));
        assert!(!root.join("public").exists());
    });
    let _ = child.kill();
    let _ = child.wait();
    result.unwrap();
}
