use notist_core::definitions::{FunctionId, ImportBinding};
use notist_core::{
    diag::Diagnostic,
    frontend::{Frontend, FrontendCacheKey, FrontendOptions, FrontendOutput},
};
use notist_pipeline::{
    BindingSpec, ContentLowering, ExportTable, Lowered, ModuleExports, Pipeline, bind_imports,
    frontend::markup::NotistFrontend,
};
use notist_syntax::content::{ContentParser, ParsedContent};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct CountingFrontend {
    configuration: &'static str,
    cached: bool,
    calls: Arc<AtomicUsize>,
}

impl Frontend for CountingFrontend {
    fn extensions(&self) -> &[&str] {
        &["not"]
    }
    fn compile(&self, source: &str, options: FrontendOptions) -> FrontendOutput {
        NotistFrontend.compile(source, options)
    }
    fn cache_key(&self) -> Option<FrontendCacheKey> {
        self.cached
            .then(|| FrontendCacheKey::new::<Self>("1", self.configuration))
    }
}
impl ContentParser for CountingFrontend {
    fn parse_content(&self, source: &str, open: rowan::TextSize) -> ParsedContent {
        self.calls.fetch_add(1, Ordering::Relaxed);
        NotistFrontend.parse_content(source, open)
    }
}
impl ContentLowering for CountingFrontend {
    fn lower_content(&self, content: &ParsedContent, diagnostics: &mut Vec<Diagnostic>) -> Lowered {
        NotistFrontend.lower_content(content, diagnostics)
    }
}

fn pipeline(configuration: &'static str, cached: bool, calls: &Arc<AtomicUsize>) -> Pipeline {
    Pipeline::default().with_frontend(CountingFrontend {
        configuration,
        cached,
        calls: calls.clone(),
    })
}

#[test]
fn fresh_pipelines_share_compiled_outputs_without_frontend_instance_identity() {
    let calls = Arc::new(AtomicUsize::new(0));
    let source = "fn show() -> InlineContent { [cached compiler test] }";
    let a = pipeline("same-config", true, &calls)
        .compile_module("content.not", "cache-fresh", "/project/lib.notc", source)
        .unwrap();
    let b = pipeline("same-config", true, &calls)
        .compile_module("content.not", "cache-fresh", "/project/lib.notc", source)
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(Arc::ptr_eq(&a.definitions, &b.definitions));
    assert!(Arc::ptr_eq(a.source(), b.source()));
}

#[test]
fn compilation_keys_include_every_context_and_the_actual_selected_frontend() {
    let calls = Arc::new(AtomicUsize::new(0));
    let source = "fn show() -> InlineContent { [compiler context test] }";
    let p = pipeline("context-config", true, &calls);
    for (id, path, root, text) in [
        ("cache-context", "/project/a.notc", "/project", source),
        ("cache-context-other", "/project/a.notc", "/project", source),
        ("cache-context", "/project/b.notc", "/project", source),
        ("cache-context", "/project/a.notc", "/", source),
        (
            "cache-context",
            "/project/a.notc",
            "/project",
            "fn show() -> InlineContent { [changed context test] }",
        ),
    ] {
        p.compile_module_with_root("content.not", id, path, text, root)
            .unwrap();
    }
    assert_eq!(calls.load(Ordering::Relaxed), 5);
    pipeline("different-config", true, &calls)
        .compile_module_with_root(
            "content.not",
            "cache-context",
            "/project/a.notc",
            source,
            "/project",
        )
        .unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 6);

    // Registration overrides must not hit the previous, cacheable implementation.
    for _ in 0..2 {
        pipeline("context-config", false, &calls)
            .compile_module_with_root(
                "content.not",
                "cache-context",
                "/project/a.notc",
                source,
                "/project",
            )
            .unwrap();
    }
    assert_eq!(calls.load(Ordering::Relaxed), 8);
}

#[test]
fn failed_compilations_are_cached_and_repaired_sources_are_recompiled() {
    let calls = Arc::new(AtomicUsize::new(0));
    let p = pipeline("error-config", true, &calls);
    let source = "fn show() -> InlineContent { [one] } fn show() -> InlineContent { [two] }";
    let a = p
        .compile_module("content.not", "cache-error", "error.notc", source)
        .unwrap_err();
    let count = calls.load(Ordering::Relaxed);
    assert!(count > 0);
    let b = p
        .compile_module("content.not", "cache-error", "error.notc", source)
        .unwrap_err();
    assert_eq!(a, b);
    assert_eq!(calls.load(Ordering::Relaxed), count);
    p.compile_module(
        "content.not",
        "cache-error",
        "error.notc",
        "fn show() -> InlineContent { [fixed] }",
    )
    .unwrap();
    assert!(calls.load(Ordering::Relaxed) > count);
}

#[test]
fn host_copy_on_write_metadata_changes_do_not_poison_cached_compilations() {
    let p = Pipeline::default();
    let source = "import \"original.notc\"; fn original() -> InlineContent;";
    let mut edited = p
        .compile_module("content.not", "cache-cow", "cow.notc", source)
        .unwrap();
    Arc::make_mut(&mut edited.definitions).functions[0].id =
        FunctionId::new("cache-cow", "changed");
    Arc::make_mut(&mut edited.imports)[0].path = "changed.notc".into();
    let fresh = p
        .compile_module("content.not", "cache-cow", "cow.notc", source)
        .unwrap();
    assert_eq!(fresh.definitions.functions[0].id.name, "original");
    assert_eq!(fresh.imports[0].path, "original.notc");
}

struct Exports(
    Arc<ExportTable>,
    notist_core::definition_identity::Definitions,
);
impl ModuleExports for Exports {
    fn function_identity(&self, id: notist_core::definitions::DefId) -> Option<FunctionId> {
        self.1.function(id).ok().cloned()
    }
    fn module_identity(
        &self,
        id: notist_core::definitions::ModuleId,
    ) -> Option<notist_core::definitions::ModuleRef> {
        self.1.module(id).ok().cloned()
    }
    fn lookup(&self, _: notist_core::definitions::ModuleId, name: &str) -> Option<ImportBinding> {
        self.0.get(name).cloned()
    }
    fn all(&self, _: notist_core::definitions::ModuleId) -> Arc<ExportTable> {
        self.0.clone()
    }
}

fn exports(names: &[(&str, &str)]) -> Exports {
    let mut identities = notist_core::definition_identity::Definitions::default();
    let table = Arc::new(
        names
            .iter()
            .map(|(name, owner)| {
                (
                    (*name).into(),
                    ImportBinding::Function(identities.insert(FunctionId::new(*owner, *name))),
                )
            })
            .collect(),
    );
    Exports(table, identities)
}

fn binding_spec(id: &str, source: &str) -> BindingSpec {
    let code = Pipeline::default()
        .compile_module("content.not", id, id, source)
        .unwrap();
    BindingSpec::new(
        code.imports.iter().map(|i| (i, "dep".into())),
        Some(&code.definitions),
    )
}

#[test]
fn member_binding_reuses_results_across_unrelated_exports_but_not_reexports() {
    let spec = binding_spec(
        "bind-cache-member",
        "import \"dep.notc\": value as selected;",
    );
    let a = bind_imports(&spec, &exports(&[("value", "first")])).unwrap();
    let b = bind_imports(
        &spec,
        &exports(&[("value", "first"), ("unrelated", "second")]),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    let c = bind_imports(&spec, &exports(&[("value", "second")])).unwrap();
    assert!(!Arc::ptr_eq(&a, &c));
    assert_eq!(
        c.imports()["selected"],
        ImportBinding::Function(FunctionId::new("second", "value").def_id())
    );
    assert!(bind_imports(&spec, &exports(&[])).is_err());
    let restored = bind_imports(&spec, &exports(&[("value", "first")])).unwrap();
    assert!(Arc::ptr_eq(&a, &restored));
}

#[test]
fn wildcard_binding_tracks_added_names_conflicts_and_repaired_missing_members() {
    let spec = binding_spec(
        "bind-cache-wildcard",
        "import \"dep.notc\": *; fn own() -> Int { 1 }",
    );
    let a = bind_imports(&spec, &exports(&[("value", "dep")])).unwrap();
    let b = bind_imports(&spec, &exports(&[("value", "dep"), ("new", "dep")])).unwrap();
    assert!(!Arc::ptr_eq(&a, &b));
    assert!(b.imports().contains_key("new"));
    assert!(
        bind_imports(&spec, &exports(&[("own", "dep")]))
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("conflict"))
    );
    let missing = binding_spec("bind-cache-missing", "import \"dep.notc\": absent;");
    let a = bind_imports(&missing, &exports(&[])).unwrap_err();
    let b = bind_imports(&missing, &exports(&[])).unwrap_err();
    assert_eq!(a, b);
    assert!(bind_imports(&missing, &exports(&[("absent", "dep")])).is_ok());
}
