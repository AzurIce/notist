#![cfg(unix)]
use std::{fs, path::Path, process::Command};

fn build(root: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(root)
        .arg("build")
        .output()
        .unwrap()
}

#[test]
fn contributed_files_publish_atomically_and_disappear_when_omitted_on_rebuild() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::create_dir(root.join("content")).unwrap();
    fs::write(root.join("content/index.not"), "= Home").unwrap();
    fs::write(root.join("Notist.toml"), "[package]\nname='test'\n[site]\noutput='public'\n[site.extensions.files]\ncommand=['python3','files.py']").unwrap();
    fs::write(
        root.join("files.py"),
        r#"
import json, sys
from pathlib import Path
for line in sys.stdin:
    r = json.loads(line)
    response = dict(version=2, event=r['event'])
    if r['event'] == 'before_template':
        response['files'] = json.loads(Path('files.json').read_text())
    print(json.dumps(response), flush=True)
"#,
    )
    .unwrap();
    fs::write(
        root.join("files.json"),
        r#"{"assets/raw.bin":{"base64":"AP8B"},".well-known/example":{"text":"hello"}}"#,
    )
    .unwrap();
    let result = build(root);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        fs::read(root.join("public/assets/raw.bin")).unwrap(),
        [0, 255, 1]
    );
    assert_eq!(
        fs::read(root.join("public/.well-known/example")).unwrap(),
        b"hello"
    );
    let original = fs::read(root.join("public/index.html")).unwrap();

    fs::write(
        root.join("files.json"),
        r#"{"index.html":{"text":"overwrite"}}"#,
    )
    .unwrap();
    let result = build(root);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("output collision"));
    assert_eq!(fs::read(root.join("public/index.html")).unwrap(), original);
    assert_eq!(
        fs::read(root.join("public/assets/raw.bin")).unwrap(),
        [0, 255, 1]
    );

    fs::write(root.join("files.json"), "{}").unwrap();
    let result = build(root);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(!root.join("public/assets/raw.bin").exists());
    assert!(!root.join("public/.well-known/example").exists());
    assert!(root.join("public/index.html").is_file());
}
