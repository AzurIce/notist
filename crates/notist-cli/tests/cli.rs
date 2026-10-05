#![cfg(not(target_arch = "wasm32"))]
use serde_json::Value;
use std::process::Command;

#[test]
fn cli_discovers_packages_inspects_modules_and_reports_config_failures() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let help = Command::new(env!("CARGO_BIN_EXE_notist"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(
        String::from_utf8(help.stdout)
            .unwrap()
            .contains("Usage: notist")
    );
    let document = root.join("docs/components/README.not");
    let query = Command::new(env!("CARGO_BIN_EXE_notist"))
        .args(["query"])
        .arg(&document)
        .arg("function:notist-doc::panel")
        .output()
        .unwrap();
    assert!(query.status.success());
    let result: Value = serde_json::from_slice(&query.stdout).unwrap();
    assert_eq!(result.as_array().unwrap().len(), 2);
    let module = Command::new(env!("CARGO_BIN_EXE_notist"))
        .arg("json")
        .arg(root.join("packages/mermaid/lib.notc"))
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

#[test]
fn html_command_publishes_used_components_and_relative_assets() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = tempfile::tempdir().unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(&root)
        .arg("html")
        .arg(root.join("docs/components/README.not"))
        .arg("--out-dir")
        .arg(output.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(output.path().join("index.html").is_file());
    assert!(
        output
            .path()
            .join("_notist/packages/notist-doc/components/panel/style.js")
            .is_file()
    );
    assert!(
        output
            .path()
            .join("_notist/packages/notist-doc/components/badge.js")
            .is_file()
    );
    let registrations = std::fs::read_to_string(output.path().join("components.js")).unwrap();
    assert_eq!(registrations.matches("customElements.define").count(), 5);
    assert!(registrations.contains("katex-math"));
    assert!(
        output
            .path()
            .join("_notist/packages/katex/components/math.js")
            .is_file()
    );
    let html = std::fs::read_to_string(output.path().join("index.html")).unwrap();
    assert_eq!(html.matches("<katex-math ").count(), 1);
    assert_eq!(html.matches("<typst-math ").count(), 2);
    assert_eq!(registrations.matches("notist-doc-panel").count(), 1);
}

#[test]
fn typst_math_replacement_exports_both_frontends_and_component_imports() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let package = root.join("packages/typst").canonicalize().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("Notist.toml"),
        format!("[dependencies]\ntypst = {{path = {package:?}}}\n"),
    )
    .unwrap();
    for (name, source, count) in [
        (
            "doc.not",
            "$frac(a, b)$ #math(\"sqrt(x)\") #typst::math(\"alpha + beta\")",
            3,
        ),
        ("doc.md", "$frac(a, b)$ and $sqrt(x)$", 2),
        (
            "doc.notmd",
            "@(id: \"formula\")\n#math(\"sqrt(x)\") $frac(a, b)$",
            2,
        ),
        ("doc.nmd", "#typst::math(\"sqrt(x)\") $frac(a, b)$", 2),
    ] {
        let source = format!(
            "{source}\n\n$ sum_(i=1)^n i $\n\n#typst::math(\"sum_(i=1)^n i\", block: true)"
        );
        let document = project.path().join(name);
        std::fs::write(&document, source).unwrap();
        let output = project.path().join(format!("{name}-html"));
        let result = Command::new(env!("CARGO_BIN_EXE_notist"))
            .current_dir(project.path())
            .arg("html")
            .arg(&document)
            .arg("--out-dir")
            .arg(&output)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let html = std::fs::read_to_string(output.join("index.html")).unwrap();
        assert_eq!(html.matches("<typst-math ").count(), count + 2);
        assert_eq!(html.matches("notist-block=\"true\"").count(), 2);
        assert!(html.contains("notist-text=\"frac(a, b)\""));
        assert!(html.contains("notist-text=\"sqrt(x)\""));
        assert!(!html.contains("<notist-math "));
        let registrations = std::fs::read_to_string(output.join("components.js")).unwrap();
        assert_eq!(registrations.matches("customElements.define").count(), 1);
        assert!(registrations.contains("typst-math"));
        for filename in ["index.js", "compiler.js"] {
            let relative = format!("components/math/{filename}");
            assert_eq!(
                std::fs::read(output.join("_notist/packages/typst").join(&relative)).unwrap(),
                std::fs::read(package.join(&relative)).unwrap()
            );
        }
    }
}

#[test]
fn checks_json_and_html_report_vault_escapes_at_the_source_reference() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("vault");
    std::fs::create_dir_all(root.join("notes")).unwrap();
    std::fs::write(root.join("README.not"), "= Home").unwrap();
    // An invalid external file must never be loaded while checking links.
    std::fs::write(temp.path().join("outside.not"), [255]).unwrap();
    let source = "[outside](../../outside.not) ![asset](../../image.svg) [inside](../README.not)";
    for extension in ["not", "md"] {
        let document = format!("notes/page.{extension}");
        std::fs::write(root.join(&document), source).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_notist"))
            .current_dir(&root)
            .args(["check", &document])
            .output()
            .unwrap();
        assert!(!result.status.success());
        let stderr = String::from_utf8(result.stderr).unwrap();
        assert_eq!(stderr.matches("outside Vault root").count(), 2, "{stderr}");
        assert!(stderr.contains(&document));
        let result = Command::new(env!("CARGO_BIN_EXE_notist"))
            .current_dir(&root)
            .args(["json", &document])
            .output()
            .unwrap();
        let output: Value = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(output["diagnostics"].as_array().unwrap().len(), 2);
        let result = Command::new(env!("CARGO_BIN_EXE_notist"))
            .current_dir(&root)
            .args(["html", &document])
            .output()
            .unwrap();
        assert!(!result.status.success());
        assert!(!root.join("target/notist-html/index.html").exists());
    }
    let result = Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(&root)
        .args(["check", "notes"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let stderr = String::from_utf8(result.stderr).unwrap();
    assert_eq!(stderr.matches("outside Vault root").count(), 4, "{stderr}");
    assert!(stderr.contains("notes/page.not") && stderr.contains("notes/page.md"));
}
