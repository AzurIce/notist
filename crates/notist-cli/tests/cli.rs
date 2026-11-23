#![cfg(not(target_arch = "wasm32"))]
use serde_json::Value;
use std::process::Command;

#[test]
fn verbose_json_preserves_machine_readable_stdout() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("doc.md"), "# Title").unwrap();
    let run = |verbose| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_notist"));
        command.current_dir(root.path()).args(["json", "doc.md"]);
        if verbose {
            command.arg("-v");
        }
        command.output().unwrap()
    };
    let normal = run(false);
    let verbose = run(true);
    assert!(normal.status.success());
    assert!(verbose.status.success());
    assert_eq!(normal.stdout, verbose.stdout);
    let parsed: Value = serde_json::from_slice(&verbose.stdout).unwrap();
    assert_eq!(parsed["diagnostics"].as_array().unwrap().len(), 0);
}

#[test]
fn read_text_and_read_json_keep_check_static_and_never_publish_private_inputs() {
    let root = tempfile::tempdir().unwrap();
    for (path, source) in [
        (
            "doc.not",
            "#import \"./view.notc\" as view\n#view.show(\"./private.txt\", \"./private.json\")",
        ),
        (
            "view.notc",
            r#"fn show(txt: String, js: String) -> Block {
                let raw = read_text(txt);
                let name = read_json(js).get("name").as_string("");
                para(text(raw + ":" + name))
            }"#,
        ),
        ("private.txt", "read body <&>"),
        (
            "private.json",
            r#"{"name":"static <&>","private":"SECRET"}"#,
        ),
    ] {
        std::fs::write(root.path().join(path), source).unwrap();
    }
    let output = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(root.path())
        .args(["html", "doc.not", "--out-dir"])
        .arg(output.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let html = std::fs::read_to_string(output.path().join("index.html")).unwrap();
    assert!(html.contains("read body &lt;&amp;&gt;:static &lt;&amp;&gt;"), "{html}");
    assert!(!html.contains("SECRET"));
    assert!(!output.path().join("private.txt").exists());
    assert!(!output.path().join("private.json").exists());

    // Removing the runtime input keeps `notist check` static and green, while `html` fails.
    std::fs::remove_file(root.path().join("private.txt")).unwrap();
    let checked = Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(root.path())
        .args(["check", "doc.not"])
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    let failed = Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(root.path())
        .args(["html", "doc.not", "--out-dir"])
        .arg(output.path())
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(
        std::fs::read_to_string(output.path().join("index.html")).unwrap(),
        html
    );
}

#[test]
#[cfg(unix)]
fn html_export_does_not_copy_symlink_or_directory_images() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("outside.svg"), "<svg/>").unwrap();
    std::fs::create_dir(root.path().join("dir.svg")).unwrap();
    std::os::unix::fs::symlink(
        root.path().join("outside.svg"),
        root.path().join("link.svg"),
    )
    .unwrap();
    std::fs::write(
        root.path().join("doc.not"),
        "![Link](./link.svg) ![Dir](./dir.svg)",
    )
    .unwrap();
    let out = tempfile::tempdir().unwrap();
    let res = Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(root.path())
        .args(["html", "doc.not", "--out-dir"])
        .arg(out.path())
        .output()
        .unwrap();
    assert!(res.status.success(), "{}", String::from_utf8_lossy(&res.stderr));
    assert!(!out.path().join("link.svg").exists());
    assert!(!out.path().join("dir.svg").exists());
}
