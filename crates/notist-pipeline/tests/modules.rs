use notist_core::{
    definitions::{FunctionId, ImportBinding},
    item::Value,
};
use notist_pipeline::{
    BindingSpec, CodeModule, Context, ExportTable, ModuleExports, Pipeline, bind_imports,
};
use std::{cell::Cell, sync::Arc};

fn compile(id: &str, source: &str) -> Arc<CodeModule> {
    Arc::new(
        Pipeline::default()
            .compile_module("content.not", id, &format!("/project/{id}.notc"), source)
            .unwrap(),
    )
}

#[test]
fn compilation_and_linking_keep_independently_shared_artifacts() {
    let dependency = compile("dep", "fn value() -> String { \"hello\" }");
    let base = Context::default().with_module(dependency.clone()).unwrap();
    let code = compile(
        "main",
        "import \"dep.notc\": value; fn show() -> InlineContent { [#text(text: value())] }",
    );
    let linked = base.link_module(code.clone(), ["dep".into()]).unwrap();
    assert!(Arc::ptr_eq(linked.code(), &code));
    assert_eq!(code.imports.len(), 1);
    assert_eq!(
        linked.scope().imports()["value"],
        ImportBinding::Function(FunctionId::new("dep", "value"))
    );
    assert_eq!(
        linked.scope().exports()["show"],
        ImportBinding::Function(FunctionId::new("main", "show"))
    );
    assert!(!linked.scope().exports().contains_key("text"));

    let installed = base.clone().with_linked_module(linked.clone()).unwrap();
    let another = base.clone().with_linked_module(linked.clone()).unwrap();
    assert!(base.program().module("main").is_none());
    for context in [&installed, &another, &installed.clone()] {
        assert!(Arc::ptr_eq(
            context.program().module("main").unwrap(),
            &linked
        ));
        assert!(std::ptr::eq(
            context.registry().function("main", "show").unwrap(),
            &code.definitions.functions[0],
        ));
        assert!(std::ptr::eq(
            context.registry().function("dep", "value").unwrap(),
            &dependency.definitions.functions[0],
        ));
        assert!(Arc::ptr_eq(
            &context.program().sources()[code.source_path()],
            code.source()
        ));
    }
    assert_eq!(
        installed.program().source_path("main"),
        Some("/project/main.notc")
    );
}

#[test]
fn unchanged_importer_uses_each_dependency_views_current_body_and_signature() {
    let pipeline = Pipeline::default();
    let code = compile(
        "main",
        "import \"dep.notc\": value; fn show() -> InlineContent { [#text(text: value())] }",
    );
    let build = |body: &str| {
        let base = Context::default()
            .with_module(compile("dep", body))
            .unwrap();
        let linked = base.link_module(code.clone(), ["dep".into()]).unwrap();
        base.with_linked_module(linked).unwrap()
    };
    let old = build("fn value() -> String { \"old\" }");
    let new = build("fn value() -> String { \"new\" }");
    let changed_type = build("fn value() -> Int { 42 }");
    let old_scope = old.program().module("main").unwrap().scope();
    assert_eq!(old_scope, new.program().module("main").unwrap().scope());
    assert_eq!(
        old_scope,
        changed_type.program().module("main").unwrap().scope()
    );
    for (context, expected) in [(&old, "old"), (&new, "new"), (&old, "old")] {
        assert!(Arc::ptr_eq(
            context.program().module("main").unwrap().code(),
            &code
        ));
        let analysis = pipeline.analyze("doc.not", "#show()", context).unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let output = pipeline.evaluate(&analysis, context);
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        assert!(
            output
                .root
                .find(|item| item.fields.get("text") == Some(&Value::Str(expected.into())))
                .is_some()
        );
    }
    let analysis = pipeline
        .analyze("doc.not", "#show()", &changed_type)
        .unwrap();
    assert!(!analysis.diagnostics().is_empty());
    assert_eq!(
        changed_type.resolve("value").unwrap().returns,
        notist_core::definitions::Type::Int
    );
}

#[test]
fn unresolved_and_failed_links_do_not_install_partial_modules() {
    let code = compile(
        "main",
        "import \"dep.notc\": missing; fn own() -> Int { 1 }",
    );
    let base = Context::default()
        .with_module(compile("dep", "fn present() -> Int { 2 }"))
        .unwrap();
    assert!(base.clone().with_module(code.clone()).is_err());
    assert!(base.link_module(code.clone(), ["absent".into()]).is_err());
    let errors = base.link_module(code, ["dep".into()]).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|d| d.message.contains("no public member `missing`"))
    );
    assert!(base.program().module("main").is_none());
    assert!(base.registry().module("main").is_none());
    assert!(
        base.clone()
            .with_module(compile("dep", "fn other() -> Int { 3 }"))
            .is_err()
    );
    assert!(base.registry().function("dep", "present").is_some());
    assert!(base.registry().function("dep", "other").is_none());
}

#[derive(Default)]
struct ObservedExports {
    lookups: Cell<usize>,
    enumerations: Cell<usize>,
    table: Arc<ExportTable>,
}

impl ModuleExports for ObservedExports {
    fn lookup(&self, module: &str, name: &str) -> Option<ImportBinding> {
        assert_eq!(module, "dep");
        self.lookups.set(self.lookups.get() + 1);
        self.table.get(name).cloned()
    }
    fn all(&self, module: &str) -> Arc<ExportTable> {
        assert_eq!(module, "dep");
        self.enumerations.set(self.enumerations.get() + 1);
        self.table.clone()
    }
}

fn spec(code: &CodeModule) -> BindingSpec {
    BindingSpec::new(
        code.imports.iter().map(|import| (import, "dep".into())),
        Some(&code.definitions),
    )
}

#[test]
fn binding_observes_only_the_export_surface_required_by_each_import_form() {
    let exports = ObservedExports {
        table: Arc::new(ExportTable::from([
            (
                "value".into(),
                ImportBinding::Function(FunctionId::new("dep", "value")),
            ),
            (
                "other".into(),
                ImportBinding::Function(FunctionId::new("dep", "other")),
            ),
        ])),
        ..Default::default()
    };
    let namespace = compile("main", "import \"dep.notc\" as dep;");
    let bound = bind_imports(&spec(&namespace), &exports).unwrap();
    assert_eq!(bound.imports()["dep"], ImportBinding::Module("dep".into()));
    assert_eq!((exports.lookups.get(), exports.enumerations.get()), (0, 0));

    let member = compile("main", "import \"dep.notc\": value as selected;");
    let bound = bind_imports(&spec(&member), &exports).unwrap();
    assert_eq!(bound.imports().len(), 1);
    assert_eq!((exports.lookups.get(), exports.enumerations.get()), (1, 0));

    let wildcard = compile("main", "import \"dep.notc\": *;");
    let bound = bind_imports(&spec(&wildcard), &exports).unwrap();
    assert_eq!(bound.imports().len(), 2);
    assert_eq!((exports.lookups.get(), exports.enumerations.get()), (1, 1));

    let absent = compile("main", "import \"dep.notc\": missing;");
    assert!(bind_imports(&spec(&absent), &exports).is_err());
    assert_eq!((exports.lookups.get(), exports.enumerations.get()), (2, 1));
}

#[test]
fn binding_specs_exclude_bodies_and_signatures_but_keep_diagnostic_locations() {
    let a = compile("main", "import \"dep.notc\": value; fn show() -> Int { 1 }");
    let b = compile("main", "import \"dep.notc\": value; fn show() -> Int { 2 }");
    assert_ne!(a, b);
    assert_eq!(spec(&a), spec(&b));

    let boolean = compile(
        "main",
        "import \"dep.notc\": value; fn show() -> Bool { true }",
    );
    let unit = compile(
        "main",
        "import \"dep.notc\": value; fn show() -> Unit { true }",
    );
    assert_ne!(boolean.definitions, unit.definitions);
    assert_eq!(spec(&boolean), spec(&unit));

    let c = compile(
        "main",
        "import \"dep.notc\": value; fn show() -> Int { 10 }",
    );
    assert_ne!(
        spec(&a),
        spec(&c),
        "moving a declaration's end still conservatively rebinds"
    );
}
