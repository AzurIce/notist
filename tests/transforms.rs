use notist::{Ctor, MemoryResources, PreparedInputs, RenderOptions, Value, Vault};
use std::collections::BTreeMap;

#[test]
fn project_diagram_declaration_selects_either_renderer_without_a_local_component() {
    for (renderer, declaration) in [
        (
            "pele",
            include_bytes!("../modules/pele/lib.notc").as_slice(),
        ),
        (
            "mermaid",
            include_bytes!("../modules/mermaid/lib.notc").as_slice(),
        ),
    ] {
        let mut inputs = MemoryResources::new("/project");
        inputs.insert(
            "components/lib.notc",
            include_bytes!("../docs/components/lib.notc").to_vec(),
        );
        inputs.insert(format!("{renderer}/lib.notc"), declaration.to_vec());
        inputs.insert(
            format!("{renderer}/diagram/index.js"),
            b"export default class extends HTMLElement {}".to_vec(),
        );
        inputs.insert(
            "Notist.toml",
            format!(
                "[[transforms]]\nkind='replace'\nimports={{components='components/lib.notc',renderer='{renderer}/lib.notc'}}\nfrom='components.diagram'\nto='renderer.diagram'"
            )
            .into_bytes(),
        );
        let semantic_id = notist::FunctionId::new("/project/components/lib.notc", "diagram");
        let target_id = notist::FunctionId::new(format!("/project/{renderer}/lib.notc"), "diagram");
        let mut vault = Vault::new(inputs);
        for (argument, theme) in [
            ("", "auto"),
            (", theme: \"light\"", "light"),
            (", theme: \"dark\"", "dark"),
        ] {
            let source = format!(
                "#import \"components/lib.notc\" as ui;\n#ui.diagram(\"flowchart LR; Source --> Item\"{argument})"
            );
            let output = vault
                .render_html("doc.not", &source, RenderOptions::default())
                .unwrap();
            assert!(output.analysis.diagnostics().is_empty());
            assert!(output.evaluation.diagnostics.is_empty());
            assert!(output.rendered.diagnostics.is_empty());
            let semantic = output
                .analysis
                .root()
                .find(|item| item.function_id() == Some(semantic_id.clone()))
                .unwrap();
            assert!(semantic.target.is_none());
            assert_eq!(
                semantic.fields.get("theme"),
                Some(&Value::Str(theme.into()))
            );
            let evaluated = output
                .evaluation
                .root
                .find(|item| item.function_id() == Some(semantic_id.clone()))
                .unwrap();
            assert_eq!(evaluated.target.as_ref().unwrap().function, target_id);
            assert_eq!(evaluated.fields, semantic.fields);
            assert_eq!(output.rendered.used_components.len(), 1);
            assert_eq!(output.rendered.used_components[0].id, target_id);
            assert!(
                output
                    .rendered
                    .html
                    .contains(&format!("notist-theme=\"{theme}\""))
            );
        }
    }
}

#[test]
fn unmapped_project_diagram_is_valid_content_and_reports_a_missing_html_target() {
    let mut inputs = MemoryResources::new("/project");
    inputs.insert(
        "components/lib.notc",
        include_bytes!("../docs/components/lib.notc").to_vec(),
    );
    let output = Vault::new(inputs)
        .render_html(
            "doc.not",
            "#import \"components/lib.notc\" as ui;\n#ui.diagram(\"flowchart LR; Source --> Item\")",
            RenderOptions::default(),
        )
        .unwrap();
    assert!(output.analysis.diagnostics().is_empty());
    assert!(output.evaluation.diagnostics.is_empty());
    assert!(output.rendered.used_components.is_empty());
    assert!(
        output
            .rendered
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("no HTML implementation"))
    );
}

fn resources(mapped: bool) -> MemoryResources {
    let mut r = MemoryResources::new("/project");
    r.insert("lib.notc", b"import \"math/view.notc\" as view;".to_vec());
    r.insert(
        "math/view.notc",
        b"fn math<C: Type = InlineContent>(text: String) -> C;".to_vec(),
    );
    r.insert("math/math.js", Vec::new());
    if mapped {
        r.insert(
            "Notist.toml",
            b"[[transforms]]\nkind='replace'\nimports={view='math/view.notc'}\nfrom='notist.math'\nto='view.math'".to_vec(),
        );
    }
    r
}
#[test]
fn importing_native_functions_does_not_implicitly_transform_content() {
    let mut v = Vault::new(resources(false));
    let a = v
        .analyze("doc.not", "#import \"math/view.notc\" as view;\n$x$")
        .unwrap();
    let e = v.evaluate("doc.not", &a).unwrap();
    let math = e.root.find(|i| i.ctor == Ctor::Math).unwrap();
    assert_eq!(math.target.as_ref().unwrap().function.module, "notist");
}
#[test]
fn explicit_transforms_resolve_private_imports_and_preserve_semantic_identity() {
    let mut v = Vault::new(resources(true));
    let output = v
        .render_html("doc.not", "$x$", RenderOptions::default())
        .unwrap();
    assert!(output.analysis.diagnostics().is_empty());
    assert!(output.evaluation.diagnostics.is_empty());
    assert!(output.rendered.diagnostics.is_empty());
    let original = output
        .analysis
        .root()
        .find(|i| i.ctor == Ctor::Math)
        .unwrap();
    assert!(original.target.is_none());
    let math = output
        .evaluation
        .root
        .find(|i| i.ctor == Ctor::Math)
        .unwrap();
    assert_eq!(math.function_id().unwrap().module, "notist");
    assert_eq!(
        math.target.as_ref().unwrap().function.module,
        "/project/math/view.notc"
    );
    assert_eq!(math.fields.get("text"), Some(&Value::Str("x".into())));
    assert_eq!(output.rendered.used_components.len(), 1);
}
#[test]
fn transform_errors_retain_the_configuration_source_and_span() {
    for target in ["missing.math", "view.unknown"] {
        let mut r = resources(false);
        let source = format!(
            "[[transforms]]\nkind='replace'\nimports={{view='math/view.notc'}}\nfrom='notist.math'\nto='{target}'"
        );
        r.insert("Notist.toml", source.as_bytes().to_vec());
        let error = Vault::new(r)
            .environment_for("doc.not", "#import \"math/view.notc\" as view;")
            .unwrap_err();
        let notist::VaultError::Environment(errors) = error else {
            panic!()
        };
        assert_eq!(errors[0].path.to_string_lossy(), "/project/Notist.toml");
        assert_eq!(errors[0].source, source);
        assert!(!errors[0].diagnostic.span.is_empty());
    }
}

#[test]
fn document_and_each_rule_have_independent_bindings_and_chained_targets() {
    let mut r = resources(false);
    for name in ["first", "second", "document"] {
        r.insert(
            format!("{name}.notc"),
            b"fn math<C: Type = InlineContent>(text: String) -> C;".to_vec(),
        );
    }
    r.insert(
        "Notist.toml",
        br#"
[[transforms]]
kind = "replace"
imports = { view = "first.notc" }
from = "notist.math"
to = "view.math"

[[transforms]]
kind = "replace"
imports = { previous = "first.notc", view = "second.notc" }
from = "previous.math"
to = "view.math"
"#
        .to_vec(),
    );
    let source = "#import \"document.notc\" as view;\n$x$ #view.math(\"direct\")";
    let mut vault = Vault::new(r);
    let analysis = vault.analyze("doc.not", source).unwrap();
    assert!(analysis.diagnostics().is_empty());
    let environment = vault.environment_for("doc.not", source).unwrap();
    assert_eq!(environment.modules().len(), 3);
    assert_eq!(
        environment
            .context()
            .resolve("view.math")
            .unwrap()
            .id
            .module,
        "/project/document.notc"
    );
    assert!(environment.context().resolve("previous.math").is_none());
    let evaluated = vault.evaluate("doc.not", &analysis).unwrap();
    assert!(evaluated.diagnostics.is_empty());
    let math = evaluated.root.find(|i| i.ctor == Ctor::Math).unwrap();
    assert_eq!(
        math.target.as_ref().unwrap().function.module,
        "/project/second.notc"
    );
    let direct = evaluated
        .root
        .find(|i| {
            i.function_id()
                .is_some_and(|id| id.module == "/project/document.notc")
        })
        .unwrap();
    assert_eq!(
        direct.target.as_ref().unwrap().function.module,
        "/project/document.notc"
    );
}

#[test]
fn rules_cannot_inherit_document_or_previous_rule_bindings() {
    for rules in [
        "[[transforms]]\nkind='replace'\nfrom='notist.math'\nto='view.math'",
        "[[transforms]]\nkind='replace'\nimports={view='math/view.notc'}\nfrom='notist.math'\nto='view.math'\n[[transforms]]\nkind='replace'\nfrom='view.math'\nto='notist.math'",
    ] {
        let mut r = resources(false);
        r.insert("Notist.toml", rules.as_bytes().to_vec());
        assert!(
            Vault::new(r)
                .analyze("doc.not", "#import \"math/view.notc\" as view;\n$x$")
                .is_err()
        );
    }
}

#[test]
fn config_paths_are_relative_to_config_and_shared_imports_are_read_once() {
    use notist::{
        Resources,
        resources::{ResourceError, ResourceKind},
    };
    use std::{
        cell::RefCell,
        path::{Path, PathBuf},
    };
    struct CountReads {
        files: MemoryResources,
        reads: RefCell<BTreeMap<PathBuf, usize>>,
    }
    impl Resources for CountReads {
        fn root(&self) -> &Path {
            self.files.root()
        }
        fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
            *self
                .reads
                .borrow_mut()
                .entry(self.resolve(path))
                .or_default() += 1;
            self.files.read(path)
        }
        fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
            self.files.kind(path)
        }
        fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
            self.files.entries(path)
        }
    }
    let mut files = resources(false);
    files.insert("config/Notist.toml", b"[[transforms]]\nkind='replace'\nimports={view='../math/view.notc'}\nfrom='notist.math'\nto='view.math'\n[[transforms]]\nkind='replace'\nimports={same='../math/../math/view.notc'}\nfrom='same.math'\nto='notist.math'".to_vec());
    let resources = CountReads {
        files,
        reads: RefCell::new(BTreeMap::new()),
    };
    let mut vault = Vault::new(&resources).with_config("config/Notist.toml");
    let analysis = vault
        .analyze(
            "notes/deep/doc.not",
            "#import \"../../math/view.notc\" as own;\n$x$",
        )
        .unwrap();
    assert!(analysis.diagnostics().is_empty());
    assert_eq!(
        resources
            .reads
            .borrow()
            .get(Path::new("/project/math/view.notc")),
        Some(&1)
    );
}

#[test]
fn transform_code_uses_its_definition_scope_and_reexported_members() {
    let mut r = MemoryResources::new("/project");
    for (path, source) in [
        ("source.notc", "fn f() -> InlineContent { [original] }"),
        ("output/index.notc", "import \"body.notc\" as nested;"),
        (
            "output/body.notc",
            "import \"value.notc\" as helper; fn f() -> InlineContent { [#text(text: helper.value())] }",
        ),
        (
            "output/value.notc",
            "fn value() -> String { \"from definition scope\" }",
        ),
        (
            "Notist.toml",
            "[[transforms]]\nkind='replace'\nimports={source='source.notc', target='output/index.notc'}\nfrom='source.f'\nto='target.nested.f'",
        ),
    ] {
        r.insert(path, source.as_bytes().to_vec());
    }
    let mut vault = Vault::new(r);
    let result = vault
        .render_html(
            "doc.not",
            "#import \"source.notc\" as target;\n#target.f()",
            Default::default(),
        )
        .unwrap();
    assert!(result.analysis.diagnostics().is_empty());
    assert!(result.evaluation.diagnostics.is_empty());
    assert!(result.rendered.html.contains("from definition scope"));
    assert!(!result.rendered.html.contains("original"));
}

#[test]
fn invalid_config_imports_and_signature_errors_retain_source_locations() {
    for config in [
        "[[transforms]]\nkind='replace'\nimports={self='math/view.notc'}\nfrom='notist.math'\nto='self.math'",
        "[[transforms]]\nkind='replace'\nimports={view='/math/view.notc'}\nfrom='notist.math'\nto='view.math'",
        "[[transforms]]\nkind='replace'\nimports={view='missing.notc'}\nfrom='notist.math'\nto='view.math'",
        "[[transforms]]\nkind='replace'\nimports={view='math/view.notc'}\nfrom='notist.text'\nto='view.math'",
    ] {
        let mut r = resources(false);
        r.insert("Notist.toml", config.as_bytes().to_vec());
        let notist::VaultError::Environment(errors) =
            Vault::new(r).analyze("doc.not", "$x$").unwrap_err()
        else {
            panic!()
        };
        assert!(
            errors
                .iter()
                .any(|e| e.path.to_str() == Some("/project/Notist.toml")
                    && !e.diagnostic.span.is_empty()
                    && e.source == config),
            "{errors:?}"
        );
    }
}
#[test]
fn prepared_worker_and_native_hosts_share_module_scopes_and_transforms() {
    let r = resources(true);
    let prepared = PreparedInputs {
        root: "/project".into(),
        config: None,
        files: r.files().clone(),
        module_urls: BTreeMap::new(),
    };
    let json: serde_json::Value = serde_json::from_str(&notist::preview::analyze_prepared(
        "doc.md",
        "$x$",
        &serde_json::to_string(&prepared).unwrap(),
    ))
    .unwrap();
    assert_eq!(json["diagnostics"], serde_json::json!([]));
    assert_eq!(json["used_components"].as_array().unwrap().len(), 1);
    assert_eq!(
        json["evaluated"]["children"][0]["children"][0]["function"],
        "notist.math"
    );
}
