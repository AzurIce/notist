use notist::{
    Context, Ctor, Environment, FunctionId, MemoryResources, Pipeline, RenderOptions, Value, Vault,
};
use std::path::Path;
fn resources(files: &[(&str, &str)]) -> MemoryResources {
    let mut r = MemoryResources::new("/project");
    for (path, text) in files {
        r.insert(path, text.as_bytes().to_vec());
    }
    r
}
fn load(files: &[(&str, &str)]) -> Environment {
    Environment::load_from(&resources(files), "lib.notc").unwrap()
}
#[test]
fn relative_imports_work_without_manifests_and_retain_definition_identity() {
    let env = load(&[
        ("lib.notc", "import \"./nested/views.notc\" as ui;"),
        (
            "nested/views.notc",
            "import \"../values.notc\" as values; fn view() -> InlineContent { [#text(text: values.message())] }",
        ),
        ("values.notc", "fn message() -> String { \"hello\" }"),
    ]);
    assert_eq!(env.modules().len(), 3);
    let p = Pipeline::default();
    for path in ["doc.not", "doc.md"] {
        let a = p.analyze(path, "#ui.view()", env.context()).unwrap();
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        let e = p.evaluate(&a, env.context());
        assert!(e.diagnostics.is_empty(), "{:?}", e.diagnostics);
        assert!(
            e.root
                .find(|i| i.fields.get("text") == Some(&Value::Str("hello".into())))
                .is_some()
        );
        assert!(
            a.root()
                .find(|i| i.function_id()
                    == Some(FunctionId::new("/project/nested/views.notc", "view")))
                .is_some()
        );
    }
}
#[test]
fn aliases_share_instances_and_normalized_paths_load_once() {
    let env = load(&[
        (
            "lib.notc",
            "import \"./view.notc\" as a; import \"./nested/../view.notc\" as b;",
        ),
        ("view.notc", "fn f() -> InlineContent { [ok] }"),
    ]);
    assert_eq!(env.modules().len(), 2);
    assert_eq!(
        env.context().resolve("a.f").unwrap().id,
        env.context().resolve("b.f").unwrap().id
    );
    let a = Pipeline::default()
        .analyze("doc.not", "#a.f() #b.f()", env.context())
        .unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let calls: Vec<_> = a
        .root()
        .descendants()
        .filter(|i| i.function_id() == Some(FunctionId::new("/project/view.notc", "f")))
        .collect();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].instance, calls[1].instance);
}
#[test]
fn same_filenames_and_function_names_in_different_directories_stay_distinct() {
    let env = load(&[
        (
            "lib.notc",
            "import \"a/lib.notc\" as a; import \"b/lib.notc\" as b;",
        ),
        ("a/lib.notc", "fn f() -> InlineContent;"),
        ("b/lib.notc", "fn f() -> InlineContent;"),
    ]);
    let a = &env.context().resolve("a.f").unwrap().id;
    let b = &env.context().resolve("b.f").unwrap().id;
    assert_ne!(a, b);
    assert_ne!(
        notist_html::components::component_tag(a),
        notist_html::components::component_tag(b)
    );
}
#[test]
fn namespaces_follow_exports_and_loaded_dependencies_are_not_ambient() {
    let env = load(&[
        ("lib.notc", "import \"a.notc\" as a;"),
        (
            "a.notc",
            "import \"b.notc\" as nested; fn own() -> InlineContent;",
        ),
        ("b.notc", "fn f() -> InlineContent;"),
    ]);
    assert!(env.context().resolve("a.nested.f").is_some());
    assert!(env.context().resolve("nested.f").is_none());
    assert!(env.context().resolve("b.f").is_none());
    assert!(env.context().resolve("own").is_none());
    assert!(env.context().resolve("notist.strong").is_some());
}
#[test]
fn function_bodies_use_their_defining_scope_and_values_shadow_module_aliases() {
    let env = load(&[
        (
            "lib.notc",
            "import \"a.notc\" as a; import \"other.notc\" as dep;",
        ),
        (
            "a.notc",
            "import \"dep.notc\" as dep; fn f() -> InlineContent { [#text(text: dep.value())] } fn bad(dep: Int) -> InlineContent { dep.f() }",
        ),
        ("dep.notc", "fn value() -> String { \"right\" }"),
        ("other.notc", "fn value() -> Int { 1 }"),
    ]);
    let p = Pipeline::default();
    let a = p.analyze("doc.not", "#a.f()", env.context()).unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let e = p.evaluate(&a, env.context());
    assert!(
        e.root
            .find(|i| i.fields.get("text") == Some(&Value::Str("right".into())))
            .is_some()
    );
    assert!(
        !p.analyze("doc.not", "#a.bad(1)", env.context())
            .unwrap()
            .diagnostics()
            .is_empty()
    );
}
#[test]
fn invalid_imports_and_cycles_have_source_located_diagnostics() {
    for files in [
        vec![("lib.notc", "import \"missing.notc\" as missing;")],
        vec![
            ("lib.notc", "import \"a.notc\" as a;"),
            ("a.notc", "import \"lib.notc\" as root;"),
        ],
        vec![("lib.notc", "import \"@registry/example:1\" as example;")],
        vec![("lib.notc", "import \"/absolute.notc\" as example;")],
        vec![(
            "lib.notc",
            "import \"x.notc\" as x; import \"y.notc\" as x;",
        )],
        vec![("lib.notc", "import \"x.notc\" as f; fn f() -> Int;")],
        vec![("lib.notc", "import \"x.notc\" as notist;")],
        vec![("lib.notc", "import name as x;")],
        vec![("lib.notc", "import \"./if.notc\";")],
        vec![("lib.notc", "import \"./types.notc\" as true;")],
    ] {
        let errors = Environment::load_from(&resources(&files), "lib.notc").unwrap_err();
        assert!(
            errors.iter().any(|e| !e.diagnostic.span.is_empty()),
            "{errors:?}"
        );
        assert!(errors.iter().all(|e| e.path.is_absolute()));
    }
}
#[test]
fn import_loading_does_not_execute_content_bodies() {
    let env = load(&[
        ("lib.notc", "import \"bad.notc\" as bad;"),
        (
            "bad.notc",
            "fn bad() -> InlineContent { compile_error(\"only when checked\") }",
        ),
    ]);
    let a = Pipeline::default()
        .analyze("doc.not", "untouched", env.context())
        .unwrap();
    assert!(a.diagnostics().is_empty());
}
#[test]
fn documents_load_only_their_explicit_imports_and_ignore_neighboring_code() {
    let r = resources(&[
        ("lib.notc", "invalid implicit code"),
        ("chosen.notc", "fn chosen() -> InlineContent { [chosen] }"),
    ]);
    let mut v = Vault::new(r);
    assert!(
        v.analyze("doc.not", "plain content")
            .unwrap()
            .diagnostics()
            .is_empty()
    );
    let source = "#import \"./chosen.notc\" as selected;\n#selected.chosen()";
    assert!(
        v.analyze("doc.not", source)
            .unwrap()
            .diagnostics()
            .is_empty()
    );
    assert!(
        v.environment_for("doc.not", source)
            .unwrap()
            .context()
            .resolve("selected.chosen")
            .is_some()
    );
    assert!(
        v.environment_for("other.not", "plain")
            .unwrap()
            .context()
            .resolve("selected.chosen")
            .is_none()
    );
    assert!(notist::environment::parse_config("context = 'chosen.notc'").is_err());
}
#[test]
fn source_overlays_load_changed_import_graphs_and_preserve_diagnostics() {
    let r = resources(&[
        ("lib.notc", "import \"old.notc\" as dep;"),
        ("old.notc", "fn f() -> Int;"),
        ("new.notc", "fn f() -> String { \"new\" }"),
    ]);
    let v = Vault::new(r);
    let source = "import \"new.notc\" as dep; fn f() -> String { dep.f() }";
    let a = v.analyze_code("lib.notc", source).unwrap();
    assert!(a.diagnostics.is_empty(), "{:?}", a.diagnostics);
    assert!(a.sources.contains_key("/project/new.notc"));
    assert!(!a.sources.contains_key("/project/old.notc"));
    assert_eq!(a.sources["/project/lib.notc"].as_ref(), source);
}
#[test]
fn native_components_are_bound_relative_to_the_defining_file() {
    let mut r = resources(&[
        ("lib.notc", "import \"parts/widgets.notc\" as ui;"),
        ("parts/widgets.notc", "fn badge() -> InlineContent;"),
    ]);
    r.insert("parts/badge.js", Vec::new());
    let mut v = Vault::new(r);
    let output = v
        .render_html(
            "doc.not",
            "#import \"./parts/widgets.notc\" as ui;\n#ui.badge()",
            RenderOptions::default(),
        )
        .unwrap();
    assert!(
        output.rendered.diagnostics.is_empty(),
        "{:?}",
        output.rendered.diagnostics
    );
    let component = &output.rendered.used_components[0];
    assert_eq!(component.id.module, "/project/parts/widgets.notc");
    assert_eq!(
        component.module.resource(),
        Some(Path::new("/project/parts/badge.js"))
    );
}
#[test]
fn pure_module_source_snapshots_use_the_same_graph_loader() {
    let mut inputs = MemoryResources::new("/");
    inputs.insert("/lib.notc", b"import \"./other.notc\" as other;".to_vec());
    inputs.insert("/other.notc", b"fn f() -> InlineContent;".to_vec());
    let env = Environment::load_from(&inputs, "/lib.notc").unwrap();
    assert_eq!(
        env.context().resolve("other.f").unwrap().id.module,
        "/other.notc"
    );
}
#[test]
fn old_qualification_and_package_manifests_are_rejected() {
    for source in [
        "[package]\nname='old'",
        "[dependencies]\na={path='a'}",
        "[dev-dependencies]\na={path='a'}",
    ] {
        assert!(notist::environment::parse_config(source).is_err());
    }
    let context = Context::default();
    let a = Pipeline::default()
        .analyze("doc.not", "#notist::strong[x]", &context)
        .unwrap();
    assert!(!a.diagnostics().is_empty());
    assert!(a.root().find(|i| i.ctor == Ctor::Strong).is_none());
}

#[test]
fn default_aliases_and_exported_namespace_chains_work_in_calls_and_bodies() {
    let env = load(&[
        (
            "lib.notc",
            "import \"./outer.notc\"; fn via() -> InlineContent { outer.inner.leaf() }",
        ),
        ("outer.notc", "import \"./leaf.notc\" as inner;"),
        (
            "leaf.notc",
            "fn leaf() -> InlineContent { [nested result] }",
        ),
    ]);
    let pipeline = Pipeline::default();
    for path in ["doc.not", "doc.md"] {
        let analysis = pipeline
            .analyze(path, "#outer.inner.leaf() #via()", env.context())
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let output = pipeline.evaluate(&analysis, env.context());
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        assert_eq!(
            output
                .root
                .descendants()
                .filter(|i| i.fields.get("text") == Some(&Value::Str("nested result".into())))
                .count(),
            2
        );
    }
    // The root source filename is not an implicit module binding.
    assert!(env.context().resolve("lib.via").is_none());
}

#[test]
fn reserved_builtin_namespace_keeps_markup_sugar_independent_of_value_bindings() {
    let env = load(&[(
        "lib.notc",
        "fn f(notist: String) -> InlineContent { [literal #text(text: notist)] }",
    )]);
    let p = Pipeline::default();
    let a = p
        .analyze("doc.not", "#f(\"argument\")", env.context())
        .unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    let evaluated = p.evaluate(&a, env.context());
    assert!(
        evaluated.diagnostics.is_empty(),
        "{:?}",
        evaluated.diagnostics
    );
    assert!(
        evaluated
            .root
            .find(|i| i.fields.get("text") == Some(&Value::Str("literal ".into())))
            .is_some()
    );
    assert!(
        evaluated
            .root
            .find(|i| i.fields.get("text") == Some(&Value::Str("argument".into())))
            .is_some()
    );
}
