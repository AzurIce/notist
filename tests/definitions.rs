use notist::builtins::Accepts;
use notist::{
    Context, DefinitionModule, FunctionDef, FunctionId, ParameterDef, ParameterMode, Pipeline,
    Registry, Type, Value, builtins,
};

#[test]
fn where_predicates_survive_definition_serialization_and_registry_snapshots() {
    let module = Pipeline::default()
        .compile_module(
            "doc.not",
            "where_wire",
            "where_wire/lib.notc",
            "fn resource<T: Type>(target: T) -> InlineContent where T == Path || T == String;",
        )
        .unwrap();
    let definition = &module.definitions.functions[0];
    let encoded = serde_json::to_value(definition).unwrap();
    assert!(encoded.get("where_clause").is_some());
    let decoded: FunctionDef = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded, *definition);
    let mut registry = Registry::new();
    registry.register(module.definitions.clone()).unwrap();
    assert_eq!(
        registry
            .function("where_wire", "resource")
            .unwrap()
            .where_clause,
        definition.where_clause
    );
    let bare = FunctionDef::new(FunctionId::new("bare", "f"), Accepts::Nothing, Type::Int);
    assert!(
        serde_json::to_value(bare)
            .unwrap()
            .get("where_clause")
            .is_none()
    );
}

#[test]
fn definitions_preserve_parameter_types_modes_and_defaults() {
    let m=notist_pipeline::Pipeline::default().compile_module("content.not", "widgets", "widgets/lib.notc","fn panel<C: Type = BlockContent>(title: String, count: Int = 2, note?: String, tags: Array<Array<Bool>> = ((true,),))[children: C] -> BlockContent;").unwrap();
    let f = &m.definitions.functions[0];
    assert_eq!(f.id, FunctionId::new("widgets", "panel"));
    assert_eq!(f.comptime.len(), 1);
    assert_eq!(f.parameters[1].mode, ParameterMode::Default(Value::Int(2)));
    assert_eq!(f.parameters[2].mode, ParameterMode::Optional);
    assert_eq!(f.children_type, Some(Type::Parameter("C".into())));
    let c = Context::default().with_module(m).unwrap();
    let a = Pipeline::default()
        .analyze("doc.not", "#self.panel(\"Title\")[ text ]", &c)
        .unwrap();
    assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
    assert_eq!(a.root().children()[0].field("count"), Some(&Value::Int(2)));
}
#[test]
fn registry_is_atomic_and_indexes_definition_modules() {
    let mut r = builtins::registry().clone();
    for module in ["one", "two"] {
        r.register(
            notist_pipeline::Pipeline::default()
                .compile_module(
                    "content.not",
                    module,
                    &format!("{}/lib.notc", module),
                    "fn diagram() -> BlockContent;",
                )
                .unwrap()
                .definitions,
        )
        .unwrap();
    }
    assert!(r.function("notist", "diagram").is_none());
    assert!(r.function("one", "diagram").is_some());
    let n = r.functions().count();
    assert!(
        r.register(
            notist_pipeline::Pipeline::default()
                .compile_module("content.not", "one", "one/lib.notc", "fn other() -> Int;")
                .unwrap()
                .definitions
        )
        .is_err()
    );
    assert_eq!(r.functions().count(), n);
    assert!(
        r.register(
            notist_pipeline::Pipeline::default()
                .compile_module(
                    "content.not",
                    "notist",
                    "notist/lib.notc",
                    "fn other() -> Int;"
                )
                .unwrap()
                .definitions
        )
        .is_err()
    );
}
#[test]
fn registry_clones_share_definitions_without_sharing_module_installations() {
    let original = builtins::registry();
    let mut first = original.clone();
    let mut second = original.clone();
    let native = FunctionId::new("notist", "text");
    assert!(std::ptr::eq(
        first.get(&native).unwrap(),
        original.get(&native).unwrap()
    ));
    let module = |name: &str| {
        Pipeline::default()
            .compile_module(
                "content.not",
                name,
                &format!("{name}/lib.notc"),
                "fn label() -> InlineContent;",
            )
            .unwrap()
            .definitions
    };
    first.register(module("one")).unwrap();
    second.register(module("two")).unwrap();
    assert!(first.function("two", "label").is_none());
    assert!(second.function("one", "label").is_none());
    assert!(original.function("one", "label").is_none());
    assert!(std::ptr::eq(
        first.get(&native).unwrap(),
        second.get(&native).unwrap()
    ));
    let mut fork = first.clone();
    fork.register(module("three")).unwrap();
    assert!(std::ptr::eq(
        first.function("one", "label").unwrap(),
        fork.function("one", "label").unwrap()
    ));
    assert!(first.function("three", "label").is_none());
    let count = fork.functions().count();
    assert!(fork.register(module("one")).is_err());
    assert!(fork.register(module("notist")).is_err());
    assert_eq!(fork.functions().count(), count);
}

#[test]
fn invalid_native_defaults_never_install_partial_modules() {
    let mut m = DefinitionModule::new("native");
    m.functions.push(FunctionDef::new(
        FunctionId::new("native", "good"),
        Accepts::Nothing,
        Type::InlineContent,
    ));
    let mut f = FunctionDef::new(
        FunctionId::new("native", "bad"),
        Accepts::Nothing,
        Type::InlineContent,
    );
    f.parameters.push(ParameterDef::new(
        "count",
        Type::Int,
        ParameterMode::Default(Value::Str("bad".into())),
    ));
    m.functions.push(f);
    let mut r = Registry::new();
    assert!(r.register(m).is_err());
    assert_eq!(r.functions().count(), 0);
}
#[test]
fn invalid_signatures_and_literals_are_located_in_code_source() {
    for source in [
        "fn a(x: Unknown)->BlockContent;",
        "fn a(x: Array<BlockContent>)->BlockContent;",
        "fn a(x: String<Int>)->BlockContent;",
        "fn a(x: Array<Int, Bool>)->BlockContent;",
        "fn a()[children: Bool]->BlockContent;",
        "fn a()->Type;",
        "fn a(x: Type)->Int;",
        "fn a(x: Int = \"bad\")->BlockContent;",
        "fn a(x: Int,x: Bool)->BlockContent;",
        "fn a()->Int; fn a()->Int;",
        "fn a(x: String = name)->BlockContent;",
        "fn a(x: Int = 9223372036854775808)->BlockContent;",
    ] {
        let errors = notist_pipeline::Pipeline::default()
            .compile_module("content.not", "pkg", "pkg/lib.notc", source)
            .unwrap_err();
        assert!(
            errors
                .iter()
                .all(|d| usize::from(d.span.end()) <= source.len()),
            "{source}: {errors:?}"
        );
    }
}
#[test]
fn shared_literals_preserve_integer_limits_raw_strings_and_dictionary_order() {
    let m=notist_pipeline::Pipeline::default().compile_module("content.not", "pkg", "pkg/lib.notc","fn f(n: Int = -9223372036854775808, data: Dict = (first: 1, second: true, first: 3), label: String = r#\"raw \\q\"#) -> BlockContent;").unwrap();
    let f = &m.definitions.functions[0];
    assert_eq!(
        f.parameters[0].mode,
        ParameterMode::Default(Value::Int(i64::MIN))
    );
    let ParameterMode::Default(Value::Dict(d)) = &f.parameters[1].mode else {
        panic!()
    };
    assert_eq!(
        d.iter().map(|(k, _)| k).collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(d.get("first"), Some(&Value::Int(3)));
    assert_eq!(
        f.parameters[2].mode,
        ParameterMode::Default(Value::Str("raw \\q".into()))
    );
}
#[test]
fn native_and_source_comptime_signatures_select_content_category() {
    let c = Context::default()
        .with_module(
            notist_pipeline::Pipeline::default()
                .compile_module(
                    "content.not",
                    "maths",
                    "maths/lib.notc",
                    "fn math<C: Type = InlineContent>(text: String) -> C;",
                )
                .unwrap(),
        )
        .unwrap();
    for (path, src, level) in [
        (
            "doc.not",
            "#self.math(\"x\")",
            notist::builtins::Level::Inline,
        ),
        (
            "doc.md",
            "#self.math<BlockContent>(\"x\")",
            notist::builtins::Level::Block,
        ),
    ] {
        let a = Pipeline::default().analyze(path, src, &c).unwrap();
        assert!(a.diagnostics().is_empty(), "{:?}", a.diagnostics());
        let i = a
            .root()
            .find(|i| i.function_id() == Some(FunctionId::new("maths", "math").def_id()))
            .unwrap();
        assert_eq!(i.level, level);
        assert!(i.field("block").is_none());
    }
}
#[test]
fn invalid_explicit_values_remain_visible_without_optional_fields() {
    let c = Context::default()
        .with_module(
            notist_pipeline::Pipeline::default()
                .compile_module(
                    "content.not",
                    "pkg",
                    "pkg/lib.notc",
                    "fn f(n: Int = 3,label?: String)->BlockContent;",
                )
                .unwrap(),
        )
        .unwrap();
    let a = Pipeline::default()
        .analyze("doc.not", "#self.f(n: \"bad\")", &c)
        .unwrap();
    assert!(!a.diagnostics().is_empty());
    let f = &a.root().children()[0];
    assert!(f.call().is_some());
    assert_eq!(f.field("n"), Some(&Value::Str("bad".into())));
    assert!(f.field("label").is_none());
}
