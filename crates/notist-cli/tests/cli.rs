#![cfg(not(target_arch = "wasm32"))]
use serde_json::Value;
use std::process::Command;

#[test]
fn cli_discovers_modules_and_reports_config_failures() {
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
        .arg(format!(
            "function:{}.panel",
            root.join("docs/components/lib.notc")
                .canonicalize()
                .unwrap()
                .display()
        ))
        .output()
        .unwrap();
    assert!(query.status.success());
    let result: Value = serde_json::from_slice(&query.stdout).unwrap();
    assert_eq!(result.as_array().unwrap().len(), 2);
    let module = Command::new(env!("CARGO_BIN_EXE_notist"))
        .arg("json")
        .arg(root.join("modules/mermaid/lib.notc"))
        .output()
        .unwrap();
    assert!(module.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&module.stdout).unwrap()["tree"]["kind"],
        "Module"
    );
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("Notist.toml");
    std::fs::write(&config, "context = 'absent.notc'").unwrap();
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
            error.contains("Notist.toml") && error.contains("unknown field"),
            "{error}"
        );
    }
}

#[test]
fn html_command_publishes_used_components_and_relative_assets() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let doc_module = root
        .join("docs/components/lib.notc")
        .canonicalize()
        .unwrap();
    let doc_key = notist_html::components::module_key(doc_module.to_str().unwrap());
    let katex_module = root.join("modules/katex/lib.notc").canonicalize().unwrap();
    let typst_module = root.join("modules/typst/lib.notc").canonicalize().unwrap();
    let katex_key = notist_html::components::module_key(katex_module.to_str().unwrap());
    let katex_tag = notist_html::components::component_tag(&notist::FunctionId::new(
        katex_module.to_str().unwrap(),
        "math",
    ));
    let typst_tag = notist_html::components::component_tag(&notist::FunctionId::new(
        typst_module.to_str().unwrap(),
        "math",
    ));
    let panel_tag = notist_html::components::component_tag(&notist::FunctionId::new(
        doc_module.to_str().unwrap(),
        "panel",
    ));
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
            .join(format!("_notist/modules/{doc_key}/panel/style.js"))
            .is_file()
    );
    assert!(
        output
            .path()
            .join(format!("_notist/modules/{doc_key}/badge.js"))
            .is_file()
    );
    let registrations = std::fs::read_to_string(output.path().join("components.js")).unwrap();
    assert_eq!(registrations.matches("customElements.define").count(), 5);
    assert!(registrations.contains(&katex_tag));
    assert!(
        output
            .path()
            .join(format!("_notist/modules/{katex_key}/math.js"))
            .is_file()
    );
    let html = std::fs::read_to_string(output.path().join("index.html")).unwrap();
    assert_eq!(html.matches(&format!("<{katex_tag} ")).count(), 1);
    assert_eq!(html.matches(&format!("<{typst_tag} ")).count(), 2);
    assert_eq!(registrations.matches(&panel_tag).count(), 1);
}

#[test]
fn typst_math_replacement_exports_both_frontends_and_component_imports() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let module = root.join("modules/typst").canonicalize().unwrap();
    let key = notist_html::components::module_key(module.join("lib.notc").to_str().unwrap());
    let tag = notist_html::components::component_tag(&notist::FunctionId::new(
        module.join("lib.notc").to_str().unwrap(),
        "math",
    ));
    let project = tempfile::tempdir_in(root.join("target")).unwrap();
    std::fs::write(
        project.path().join("lib.notc"),
        "import \"../../modules/typst/lib.notc\" as typst;",
    )
    .unwrap();
    std::fs::write(
        project.path().join("Notist.toml"),
        "[[transforms]]\nkind='replace'\nimports={typst='../../modules/typst/lib.notc'}\nfrom='notist.math'\nto='typst.math'\n",
    )
    .unwrap();
    for (name, source, count) in [
        (
            "doc.not",
            "$frac(a, b)$ #math(\"sqrt(x)\") #typst.math(\"alpha + beta\")",
            3,
        ),
        ("doc.md", "$frac(a, b)$ and $sqrt(x)$", 2),
        (
            "doc.notmd",
            "@(id: \"formula\")\n#math(\"sqrt(x)\") $frac(a, b)$",
            2,
        ),
        ("doc.nmd", "#typst.math(\"sqrt(x)\") $frac(a, b)$", 2),
    ] {
        let source = format!(
            "#import \"../../modules/typst/lib.notc\" as typst;\n{source}\n\n$ sum_(i=1)^n i $\n\n#typst.math<BlockContent>(\"sum_(i=1)^n i\")"
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
        assert_eq!(html.matches(&format!("<{tag} ")).count(), count + 2);
        assert_eq!(html.matches("notist-content=\"block\"").count(), 2);
        assert!(html.contains("notist-text=\"frac(a, b)\""));
        assert!(html.contains("notist-text=\"sqrt(x)\""));
        assert!(!html.contains("<notist-math "));
        let registrations = std::fs::read_to_string(output.join("components.js")).unwrap();
        assert_eq!(registrations.matches("customElements.define").count(), 1);
        assert!(registrations.contains(&tag));
        for filename in ["index.js", "compiler.js"] {
            let relative = format!("math/{filename}");
            assert_eq!(
                std::fs::read(
                    output
                        .join(format!("_notist/modules/{key}"))
                        .join(&relative)
                )
                .unwrap(),
                std::fs::read(module.join(&relative)).unwrap()
            );
        }
    }
}

#[test]
fn typst_svg_exports_source_and_worker_resources_without_using_math() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let module = root.join("modules/typst").canonicalize().unwrap();
    let key = notist_html::components::module_key(module.join("lib.notc").to_str().unwrap());
    let tag = notist_html::components::component_tag(&notist::FunctionId::new(
        module.join("lib.notc").to_str().unwrap(),
        "svg",
    ));
    let project = tempfile::tempdir_in(root.join("target")).unwrap();
    let source = r##"#import "../../modules/typst/lib.notc" as typst;
#typst.svg(r#"""
#import "@preview/fletcher:0.5.8": diagram, node
#diagram(node((0, 0), [中文 <&>]))
"""#)"##;
    for name in ["doc.not", "doc.md"] {
        let document = project.path().join(name);
        std::fs::write(&document, source).unwrap();
        let output = project.path().join(format!("{name}-html"));
        let result = Command::new(env!("CARGO_BIN_EXE_notist"))
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
        assert_eq!(html.matches(&format!("<{tag} ")).count(), 1);
        assert!(html.contains("notist-source=\"#import &quot;@preview/fletcher:0.5.8&quot;"));
        assert!(html.contains("中文 &lt;&amp;&gt;"));
        let registrations = std::fs::read_to_string(output.join("components.js")).unwrap();
        assert_eq!(registrations.matches("customElements.define").count(), 1);
        assert!(registrations.contains(&tag));
        let resources = output.join(format!("_notist/modules/{key}"));
        for filename in ["index.js", "runtime.js", "compiler.js"] {
            assert_eq!(
                std::fs::read(resources.join("svg").join(filename)).unwrap(),
                std::fs::read(module.join("svg").join(filename)).unwrap(),
            );
        }
        assert!(!resources.join("math").exists());
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

#[test]
fn eval_inspection_exposes_code_output_and_definition_source() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(root.join("Notist.toml"), "").unwrap();
    let definition = "fn f() -> InlineContent { [#strong[body]] }";
    std::fs::write(root.join("lib.notc"), definition).unwrap();
    std::fs::write(
        root.join("doc.not"),
        "#import \"lib.notc\" as demo;\n#demo.f()",
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_notist"))
            .current_dir(root)
            .args(args)
            .output()
            .unwrap()
    };
    let semantic = run(&["query", "doc.not", "function:notist.strong"]);
    assert!(semantic.status.success());
    let v: Value = serde_json::from_slice(&semantic.stdout).unwrap();
    assert!(v.as_array().unwrap().is_empty());
    let evaluated = run(&["query", "--eval", "doc.not", "function:notist.strong"]);
    assert!(
        evaluated.status.success(),
        "{}",
        String::from_utf8_lossy(&evaluated.stderr)
    );
    let v: Value = serde_json::from_slice(&evaluated.stdout).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1);
    assert!(v[0]["text"].as_str().unwrap().contains("#strong[body]"));
    assert!(
        v[0]["origin"]["source"]
            .as_str()
            .unwrap()
            .ends_with("lib.notc")
    );
    let output = run(&["core", "--eval", "doc.not"]);
    assert!(String::from_utf8_lossy(&output.stdout).contains("strong"));
    let output = run(&["json", "--eval", "doc.not"]);
    let v: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(v["core"].to_string().contains("expanded"));
}

#[test]
fn module_check_and_json_use_the_actual_source_and_imports() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir(root.join("dep")).unwrap();
    for (path, source) in [
        ("Notist.toml", ""),
        ("dep/Notist.toml", ""),
        ("dep/lib.notc", "fn value() -> String { \"dependency\" }"),
        (
            "lib.notc",
            "import \"dep/lib.notc\" as dep; fn helper() -> String { dep.value() } fn view() -> InlineContent { [#text(text: self.helper())] }",
        ),
    ] {
        std::fs::write(root.join(path), source).unwrap();
    }
    for command in ["check", "json"] {
        let result = Command::new(env!("CARGO_BIN_EXE_notist"))
            .current_dir(root)
            .args([command, "lib.notc"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        if command == "json" {
            let data: Value = serde_json::from_slice(&result.stdout).unwrap();
            assert_eq!(
                data["functions"][1]["function"],
                format!("{}.view", root.join("lib.notc").display())
            );
            assert!(data["diagnostics"].as_array().unwrap().is_empty());
        }
    }
    std::fs::write(
        root.join("lib.notc"),
        "fn f() -> InlineContent { compile_error(\"module problem\") }",
    )
    .unwrap();
    std::fs::write(
        root.join("doc.not"),
        "#import \"lib.notc\" as demo;\n#demo.f()  中文中文中文中文",
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(root)
        .args(["check", "."])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let error = String::from_utf8_lossy(&result.stderr);
    assert!(error.contains("lib.notc:1:"), "{error}");
    assert!(!error.contains("doc.not:1:"), "{error}");
}

#[test]
fn source_imports_load_a_code_module_without_context_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::write(
        root.join("selected.notc"),
        "fn mark() -> InlineContent { [selected module] }",
    )
    .unwrap();
    std::fs::write(root.join("lib.notc"), "invalid implicit module").unwrap();
    std::fs::write(
        root.join("doc.not"),
        "#import \"selected.notc\" as selected;\n#selected.mark()",
    )
    .unwrap();
    for command in ["check", "core", "json", "html"] {
        let mut cli = Command::new(env!("CARGO_BIN_EXE_notist"));
        cli.current_dir(root).args([command, "doc.not"]);
        if command == "core" || command == "json" {
            cli.arg("--eval");
        }
        let output = cli.output().unwrap();
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if command == "core" || command == "json" {
            assert!(String::from_utf8_lossy(&output.stdout).contains("selected module"));
        }
    }
    let removed = Command::new(env!("CARGO_BIN_EXE_notist"))
        .current_dir(root)
        .args(["check", "doc.not", "--context", "selected.notc"])
        .output()
        .unwrap();
    assert!(!removed.status.success());
}
