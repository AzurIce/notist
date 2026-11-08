use notist::{
    DefId, DefinitionModule, FunctionDef, FunctionId, ModuleId, ModuleRef, Type, builtins::Accepts,
};
use notist_core::registry::Registry;

fn registry(returns: Type) -> Registry {
    let mut module = DefinitionModule::new("identity-snapshot");
    module.functions.push(FunctionDef::new(
        FunctionId::new("identity-snapshot", "panel"),
        Accepts::Nothing,
        returns,
    ));
    let mut registry = Registry::new();
    registry.register(module).unwrap();
    registry
}

#[test]
fn numeric_ids_are_compact_but_never_stand_in_for_registry_revisions() {
    assert_eq!(size_of::<DefId>(), 4);
    assert_eq!(size_of::<ModuleId>(), 4);
    let inline = registry(Type::InlineContent);
    let block = registry(Type::BlockContent);
    let function = FunctionId::new("identity-snapshot", "panel");
    assert_eq!(
        inline.get(&function).unwrap().id.def_id(),
        block.get(&function).unwrap().id.def_id()
    );
    assert_eq!(
        inline.definition(function.def_id()).unwrap().returns,
        Type::InlineContent
    );
    assert_eq!(
        block.definition(function.def_id()).unwrap().returns,
        Type::BlockContent
    );
    assert_eq!(
        inline.clone().get(&function).unwrap().returns,
        Type::InlineContent
    );
    let foreign = FunctionId::new("identity-other-snapshot", "panel");
    assert!(inline.get(&foreign).is_none());
    assert!(inline.definition(foreign.def_id()).is_none());
}

#[test]
fn module_ids_include_empty_modules_and_snapshot_clones_remain_independent() {
    let mut original = registry(Type::InlineContent);
    let cloned = original.clone();
    let empty = ModuleRef::new("identity-empty-module");
    original
        .register(DefinitionModule::new(empty.name()))
        .unwrap();
    assert_eq!(
        original.module_by_id(empty.id()).unwrap().module,
        empty.name()
    );
    assert!(cloned.module_by_id(empty.id()).is_none());
}

#[test]
fn escaping_identity_owns_names_and_wire_contains_no_process_local_ids() {
    let function = {
        let registry = registry(Type::InlineContent);
        registry
            .function("identity-snapshot", "panel")
            .unwrap()
            .id
            .clone()
    };
    assert_eq!(function.to_string(), "identity-snapshot.panel");
    let wire = serde_json::to_value(&function).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({"module": "identity-snapshot", "name": "panel"})
    );
    let imported: FunctionId = serde_json::from_value(wire).unwrap();
    assert_eq!(imported, function);
    assert_eq!(imported.def_id(), function.def_id());
    let unicode = FunctionId::new("包", "图");
    assert_eq!(
        serde_json::from_value::<FunctionId>(serde_json::to_value(&unicode).unwrap()).unwrap(),
        unicode
    );
}

#[test]
fn public_order_and_debug_are_independent_of_numeric_allocation_order() {
    let z = FunctionId::new("identity-sort", "z");
    let a = FunctionId::new("identity-sort", "a");
    let mut functions = vec![z, a];
    functions.sort();
    assert_eq!(functions[0].name, "a");
    assert_eq!(
        format!("{:?}", functions[0]),
        "FunctionId { module: \"identity-sort\", name: \"a\" }"
    );
}

#[test]
fn equal_logical_ids_in_distinct_contexts_do_not_reuse_checked_implementations() {
    let pipeline = notist::Pipeline::default();
    let context = |text| {
        let module = pipeline
            .compile_module("doc.not", "identity-code", "identity-code.notc", text)
            .unwrap();
        notist::Context::default().with_module(module).unwrap()
    };
    let first = context("fn show() -> InlineContent { [first] }");
    let second = context("fn show() -> InlineContent { [second] }");
    let id = FunctionId::new("identity-code", "show");
    assert_eq!(
        first.registry().get(&id).unwrap().id.def_id(),
        second.registry().get(&id).unwrap().id.def_id()
    );
    for (context, expected) in [(&first, "first"), (&second, "second"), (&first, "first")] {
        let analysis = pipeline
            .analyze("doc.not", "#self.show()", context)
            .unwrap();
        let result = pipeline.evaluate(&analysis, context);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(notist_html::render(&result.root).contains(expected));
    }
}

#[test]
fn resolved_ir_is_numeric_and_escaping_trees_use_explicit_name_ownership() {
    let pipeline = notist::Pipeline::default();
    let module = pipeline
        .compile_module(
            "doc.not",
            "numeric-tree",
            "numeric-tree.notc",
            "fn panel() -> InlineContent;",
        )
        .unwrap();
    let context = notist::Context::default().with_module(module).unwrap();
    let id = context.resolve("self.panel").unwrap().id.def_id();
    let analysis = pipeline
        .analyze("doc.not", "#self.panel()", &context)
        .unwrap();
    assert!(analysis.diagnostics().is_empty());
    let (root, _, sources) = analysis.into_parts();
    drop(context);
    let call = root.find(|node| node.function_id() == Some(id)).unwrap();
    assert_eq!(size_of_val(&call.call().unwrap().instance.function), 4);
    let notist::Ctor::Extension(constructor) = call.ctor() else {
        panic!("external constructor")
    };
    assert_eq!(size_of_val(&constructor.id), 4);
    sources.validate_node(&root).unwrap();
    assert_eq!(
        sources.definitions().function(id).unwrap().to_string(),
        "numeric-tree.panel"
    );
    assert!(
        notist::dump::dump(&root, &sources)
            .unwrap()
            .contains("numeric-tree.panel")
    );
    assert_eq!(
        notist::query::select(&root, "function:numeric-tree.panel", &sources)
            .unwrap()
            .len(),
        1
    );
    let wire = serde_json::to_value(sources.wire(&root)).unwrap();
    assert!(wire.to_string().contains("numeric-tree"));
    assert!(!wire.to_string().contains("DefId"));
    let mut source_only = notist::Sources::new();
    for source in sources.records() {
        source_only.insert(source.clone()).unwrap();
    }
    assert!(source_only.validate_node(&root).is_err());
    assert!(serde_json::to_value(source_only.wire(&root)).is_err());
    assert!(source_only.decode::<notist::Node>(wire.clone()).is_err());
    assert!(notist::query::select(&root, "function:numeric-tree.panel", &source_only).is_err());
    let imported = notist::Sources::import::<notist::Node>(
        wire.clone(),
        notist::SourceNamespace::named("numeric-tree-import"),
    )
    .unwrap();
    assert!(imported.sources.validate_node(&imported.value).is_ok());
    assert_eq!(
        serde_json::to_value(imported.sources.wire(&imported.value)).unwrap(),
        wire
    );
}

#[test]
fn linked_imports_and_host_module_lookups_use_module_ids_with_owned_names() {
    let pipeline = notist::Pipeline::default();
    let dependency = pipeline
        .compile_module(
            "doc.not",
            "numeric-dep",
            "dep.notc",
            "fn value() -> Int { 7 }",
        )
        .unwrap();
    let dependency_id = dependency.module_id();
    let context = notist::Context::default().with_module(dependency).unwrap();
    let module = pipeline
        .compile_module(
            "doc.not",
            "numeric-main",
            "main.notc",
            "import \"dep.notc\" as dep; fn show() -> Int { dep.value() }",
        )
        .unwrap();
    let linked = context
        .link_module_resolved(module, [dependency_id])
        .unwrap();
    let scope = linked.scope().clone();
    let notist::ImportBinding::Module(imported_id) = scope.imports()["dep"] else {
        panic!("namespace")
    };
    assert_eq!(size_of_val(&imported_id), 4);
    assert_eq!(imported_id, dependency_id);
    let context = context.with_linked_module(linked).unwrap();
    assert_eq!(
        context.root_id(),
        context
            .program()
            .module("numeric-main")
            .map(|module| module.code().module_id())
    );
    drop(context);
    assert_eq!(
        scope.identities().module(imported_id).unwrap().name(),
        "numeric-dep"
    );
}

#[test]
fn cow_module_relocation_reindexes_bodies_without_changing_the_compiled_original() {
    let pipeline = notist::Pipeline::default();
    let original = pipeline
        .compile_module(
            "doc.not",
            "numeric-original",
            "original.notc",
            "fn value() -> InlineContent { [retained body] }",
        )
        .unwrap();
    let original_id = original.module_id();
    let mut relocated = original.clone();
    let definitions = std::sync::Arc::make_mut(&mut relocated.definitions);
    definitions.module = ModuleRef::new("numeric-relocated");
    for definition in &mut definitions.functions {
        definition.id = FunctionId::new("numeric-relocated", &definition.id.name);
    }
    assert_ne!(original_id, relocated.module_id());
    assert_eq!(original.module_id(), original_id);
    for module in [original, relocated] {
        let context = notist::Context::default().with_module(module).unwrap();
        let analysis = pipeline
            .analyze("doc.not", "#self.value()", &context)
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let result = pipeline.evaluate(&analysis, &context);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        assert!(
            notist_html::Renderer::new()
                .with_sources(&result.sources)
                .render(&result.root)
                .contains("retained body")
        );
    }
}

#[test]
fn html_numeric_targets_retain_names_after_the_semantic_registry_is_dropped() {
    use notist_html::components::{HtmlRegistry, ModuleLocator};
    use std::collections::BTreeMap;
    let mut module = DefinitionModule::new("numeric-html");
    for name in ["z", "a"] {
        module.functions.push(FunctionDef::new(
            FunctionId::new("numeric-html", name),
            Accepts::Nothing,
            Type::InlineContent,
        ));
    }
    let id = module.functions[0].id.def_id();
    let mut registry = Registry::new();
    registry.register(module).unwrap();
    let bindings: BTreeMap<DefId, ModuleLocator> = registry
        .functions()
        .map(|definition| (definition.id.def_id(), ModuleLocator::from("module.js")))
        .collect();
    let html = HtmlRegistry::from_bindings(&registry, &bindings).unwrap();
    drop(registry);
    assert!(html.target(id).is_some());
    assert_eq!(
        html.identities().function(id).unwrap().to_string(),
        "numeric-html.z"
    );
    let node = notist::Node::new(
        notist::Ctor::Extension(notist_core::item::ExtensionCtor {
            id,
            accepts: Accepts::Nothing,
            level: notist::builtins::Level::Inline,
        }),
        Default::default(),
    );
    let sources = notist::Sources::new().with_definitions(html.identities());
    let wire = serde_json::to_value(sources.wire(&node)).unwrap();
    assert_eq!(
        wire["content"]["Item"]["ctor"]["Extension"]["id"],
        serde_json::json!({"module":"numeric-html","name":"z"})
    );
    let rendered = notist_html::Renderer::from_registry(html)
        .with_sources(&sources)
        .render_with_diagnostics(&node);
    assert!(
        rendered.diagnostics.is_empty(),
        "{:?}",
        rendered.diagnostics
    );
    assert!(rendered.html.contains("numeric-html.z"));
}

#[test]
fn host_environment_indexes_modules_numerically_but_presents_names_lexically() {
    let mut resources = notist::MemoryResources::new("/numeric-host");
    resources.insert(
        "z.notc",
        b"import \"a.notc\" as dep; fn value() -> Int { dep.value() }".to_vec(),
    );
    resources.insert("a.notc", b"fn value() -> Int { 7 }".to_vec());
    let environment = notist::Environment::load_from(&resources, "z.notc").unwrap();
    let _: &std::collections::BTreeMap<ModuleId, notist::ModuleSource> = environment.modules();
    let names: Vec<_> = environment.module_records().map(|(name, _)| name).collect();
    assert_eq!(names, ["/numeric-host/a.notc", "/numeric-host/z.notc"]);
    for (id, source) in environment.modules() {
        assert_eq!(
            environment
                .context()
                .identities()
                .module(*id)
                .unwrap()
                .name(),
            source.path.to_str().unwrap()
        );
    }
}

#[test]
fn scope_installation_rejects_ids_known_globally_but_not_owned_by_this_context() {
    let context = notist::Context::default();
    let foreign = FunctionId::new("numeric-foreign-context", "value");
    let foreign_module = ModuleRef::new("numeric-foreign-context");
    assert!(
        context
            .clone()
            .with_scope(
                "doc",
                [(
                    "foreign".into(),
                    notist::ImportBinding::Function(foreign.def_id())
                )]
            )
            .is_err()
    );
    assert!(
        context
            .clone()
            .with_scope(
                "doc",
                [(
                    "foreign".into(),
                    notist::ImportBinding::Module(foreign_module.id())
                )]
            )
            .is_err()
    );
    assert!(context.registry().definition(foreign.def_id()).is_none());
    assert!(
        context
            .registry()
            .module_by_id(foreign_module.id())
            .is_none()
    );
}

#[test]
fn wire_and_validation_reject_a_call_whose_instance_and_contract_ids_disagree() {
    let pipeline = notist::Pipeline::default();
    let module = pipeline
        .compile_module(
            "doc.not",
            "numeric-wire",
            "wire.notc",
            "fn show() -> InlineContent; fn other() -> InlineContent;",
        )
        .unwrap();
    let context = notist::Context::default().with_module(module).unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.show()", &context)
        .unwrap();
    let original = analysis.root().find(|node| node.call().is_some()).unwrap();
    let mut wire = serde_json::to_value(analysis.sources().wire(original)).unwrap();
    wire["content"]["Expr"]["kind"]["Call"]["instance"]["function"] =
        serde_json::json!({"module":"numeric-wire","name":"other"});
    assert!(analysis.sources().decode::<notist::Node>(wire).is_err());
    let mut edited = original.clone();
    edited.call_mut().unwrap().instance.function =
        context.resolve("self.other").unwrap().id.def_id();
    assert!(analysis.sources().validate_node(&edited).is_err());
    assert!(serde_json::to_value(analysis.sources().wire(&edited)).is_err());
}
