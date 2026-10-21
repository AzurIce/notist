use notist::builtins::Accepts;
use notist::{
    Context, DefinitionModule, FunctionDef, FunctionId, ParameterDef, ParameterMode, Pipeline,
    Registry, Type, Value, builtins,
};
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
    assert_eq!(
        a.root().children[0].fields.get("count"),
        Some(&Value::Int(2))
    );
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
            .find(|i| i.function_id() == Some(FunctionId::new("maths", "math")))
            .unwrap();
        assert_eq!(i.level, level);
        assert!(i.fields.get("block").is_none());
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
    let f = &a.root().children[0];
    assert_eq!(f.fields.get("n"), Some(&Value::Str("bad".into())));
    assert!(f.fields.get("label").is_none());
}
