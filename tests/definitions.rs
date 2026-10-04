use notist::builtins::{Accepts, Level};
use notist::{
    DefinitionModule, Dict, FunctionDef, FunctionId, ParameterDef, ParameterMode, Registry,
    ReturnRule, TextRange, Value, ValueType, analyze_module, builtins,
};

fn span() -> TextRange {
    TextRange::empty(0.into())
}

#[test]
fn source_definitions_preserve_parameter_modes_types_and_defaults() {
    let module = analyze_module("widgets", "fn panel(title: String, count: Int = 2, note?: String, tags: Array<Array<Bool>> = ((true,),))[children: Content] -> Content;").unwrap();
    let definition = &module.functions[0];
    assert_eq!(definition.id, FunctionId::new("widgets", "panel"));
    assert_eq!(definition.children, Accepts::Content);
    assert_eq!(definition.returns, ReturnRule::Fixed(Level::Block));
    assert_eq!(
        definition.parameters[1].mode,
        ParameterMode::Default(Value::Int(2))
    );
    assert_eq!(definition.parameters[2].mode, ParameterMode::Optional);
    assert!(definition.parameters.iter().all(|p| p.positional));
    let mut registry = builtins::registry().clone();
    registry.register(module).unwrap();
    let mut diagnostics = Vec::new();
    let fields = registry.resolve("widgets::panel").unwrap().bind_fields(
        [Value::Str("Title".into())],
        Dict::default(),
        "widgets::panel",
        span(),
        &mut diagnostics,
    );
    assert!(diagnostics.is_empty());
    assert_eq!(fields.get("title"), Some(&Value::Str("Title".into())));
    assert_eq!(fields.get("count"), Some(&Value::Int(2)));
    assert!(fields.get("note").is_none());
    assert_eq!(
        fields.get("tags"),
        Some(&Value::Array(vec![Value::Array(vec![Value::Bool(true)])]))
    );
}

#[test]
fn native_and_source_definitions_share_validation_and_call_binding() {
    let source = analyze_module("copy", "fn math(text: String) -> InlineContent;").unwrap();
    let mut native = DefinitionModule::new("copy");
    let mut function = builtins::registry().resolve("math").unwrap().clone();
    function.id.package = "copy".into();
    native.functions.push(function);
    let mut native_registry = builtins::registry().clone();
    let mut source_registry = builtins::registry().clone();
    native_registry.register(native).unwrap();
    source_registry.register(source).unwrap();
    for values in [
        vec![],
        vec![Value::Str("x".into())],
        vec![Value::Int(1)],
        vec![Value::Str("x".into()), Value::Bool(false)],
    ] {
        let mut native_diags = Vec::new();
        let mut source_diags = Vec::new();
        let native_fields = native_registry.resolve("copy::math").unwrap().bind_fields(
            values.clone(),
            Dict::default(),
            "copy::math",
            span(),
            &mut native_diags,
        );
        let source_fields = source_registry.resolve("copy::math").unwrap().bind_fields(
            values,
            Dict::default(),
            "copy::math",
            span(),
            &mut source_diags,
        );
        assert_eq!(native_fields, source_fields);
        assert_eq!(native_diags, source_diags);
    }
}

#[test]
fn registry_keeps_package_scope_and_builtin_prelude_distinct() {
    let mut registry = builtins::registry().clone();
    registry
        .register(analyze_module("one", "fn diagram() -> Content;").unwrap())
        .unwrap();
    registry
        .register(analyze_module("two", "fn diagram() -> InlineContent;").unwrap())
        .unwrap();
    assert_eq!(
        registry.resolve("callout").unwrap(),
        registry.resolve("notist::callout").unwrap()
    );
    assert_eq!(registry.resolve("one::diagram").unwrap().id.package, "one");
    assert_eq!(registry.resolve("two::diagram").unwrap().id.package, "two");
    assert!(registry.resolve("diagram").is_err());
    assert_eq!(
        registry.resolve("one::sub::diagram").unwrap_err(),
        notist::registry::LookupError::UnsupportedPath
    );
    let count = registry.functions().count();
    assert!(
        registry
            .register(analyze_module("one", "fn another() -> Content;").unwrap())
            .is_err()
    );
    assert_eq!(registry.functions().count(), count);
    assert!(registry.resolve("one::another").is_err());
    assert!(
        registry
            .register(analyze_module("notist", "fn shadow() -> Content;").unwrap())
            .is_err()
    );
}

#[test]
fn invalid_native_module_is_never_partially_installed() {
    let mut module = DefinitionModule::new("native");
    module.functions.push(FunctionDef::new(
        FunctionId::new("native", "good"),
        Accepts::Nothing,
        ReturnRule::Fixed(Level::Inline),
    ));
    let mut bad = FunctionDef::new(
        FunctionId::new("native", "bad"),
        Accepts::Nothing,
        ReturnRule::Fixed(Level::Inline),
    );
    bad.parameters.push(ParameterDef::new(
        "count",
        ValueType::Int,
        ParameterMode::Default(Value::Str("bad".into())),
    ));
    module.functions.push(bad);
    let mut registry = Registry::new();
    let diagnostics = registry.register(module).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.message.contains("default for `count`"))
    );
    assert_eq!(registry.functions().count(), 0);
    assert!(registry.resolve("native::good").is_err());
}

#[test]
fn definition_errors_are_located_in_code_source() {
    for (source, message) in [
        ("fn a(x: Unknown)->Content;", "unknown value type"),
        ("fn a(x: Content)->Content;", "mounted as children"),
        ("fn a(x: Array<Content>)->Content;", "mounted as children"),
        ("fn a(x: String<Int>)->Content;", "takes no type arguments"),
        (
            "fn a(x: Array<Int, Bool>)->Content;",
            "at most one element type",
        ),
        (
            "fn a(x: String)[children: Bool]->Content;",
            "expected Content or InlineContent",
        ),
        ("fn a()->String;", "expected Content or InlineContent"),
        ("fn a(x: Int = \"bad\")->Content;", "default for `x`"),
        (
            "fn a(x: Array<Array<Bool>> = ((1,),))->Content;",
            "default for `x`",
        ),
        ("fn a(x: Int, x: Bool)->Content;", "duplicate parameter"),
        ("fn a()->Content; fn a()->Content;", "duplicate function"),
        (
            "fn a(x: String = name)->Content;",
            "bare names are not literals",
        ),
        ("fn a(x: String = \"\\q\")->Content;", "unknown escape"),
        (
            "fn a(x: Int = 9223372036854775808)->Content;",
            "number out of range",
        ),
        ("fn a(x: Dict = (key: 1, 2))->Content;", "dict members"),
    ] {
        let diagnostics = analyze_module("pkg", source).unwrap_err();
        let error = diagnostics
            .iter()
            .find(|d| d.message.contains(message))
            .unwrap_or_else(|| panic!("{source}: {diagnostics:?}"));
        assert!(error.span.start() < error.span.end(), "{source}: {error:?}");
        assert!(u32::from(error.span.end()) as usize <= source.len());
    }
    assert!(
        analyze_module("pkg", "fn broken(\nfn good()->Content;")
            .unwrap_err()
            .iter()
            .any(|d| d.phase == notist::Phase::Syntax)
    );
}

#[test]
fn literal_conversion_is_shared_and_preserves_integer_range_and_dict_order() {
    let code = "fn config(n: Int = -9223372036854775808, data: Dict = (first: 1, second: true, first: 3), label: String = r#\"raw \\q\"#) -> Content;";
    let module = analyze_module("pkg", code).unwrap();
    let mut diags = Vec::new();
    let defaults =
        module.functions[0].bind_fields([], Dict::default(), "pkg::config", span(), &mut diags);
    assert_eq!(defaults.get("n"), Some(&Value::Int(i64::MIN)));
    let Some(Value::Dict(data)) = defaults.get("data") else {
        panic!("expected dict");
    };
    assert_eq!(
        data.iter().map(|(key, _)| key).collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(data.get("first"), Some(&Value::Int(3)));
    assert_eq!(defaults.get("label"), Some(&Value::Str("raw \\q".into())));
    let document = notist::analyze("#list(start: -9223372036854775808)[]");
    assert!(document.1.is_empty(), "{:?}", document.1);
    assert_eq!(
        document.0.children[0].fields.get("start"),
        Some(&Value::Int(i64::MIN))
    );
}

#[test]
fn explicit_invalid_values_are_retained_and_optional_values_are_not_inserted() {
    let module = analyze_module("pkg", "fn f(n: Int = 3, label?: String)->Content;").unwrap();
    let mut fields = Dict::default();
    fields.insert("n", Value::Str("invalid".into()));
    fields.insert("extra", Value::Bool(true));
    let mut diags = Vec::new();
    let fields = module.functions[0].bind_fields([], fields, "pkg::f", span(), &mut diags);
    assert_eq!(diags.len(), 1);
    assert_eq!(fields.get("n"), Some(&Value::Str("invalid".into())));
    assert!(fields.get("label").is_none());
    assert_eq!(fields.get("extra"), Some(&Value::Bool(true)));
}

#[test]
fn native_dynamic_and_structural_contracts_are_checked_at_registration() {
    let raw = builtins::registry().resolve("raw").unwrap();
    let mut fields = Dict::default();
    assert_eq!(
        raw.returns.level(&fields, notist::expr::BodyFlavor::None),
        Level::Inline
    );
    fields.insert("block", Value::Bool(true));
    assert_eq!(
        raw.returns.level(&fields, notist::expr::BodyFlavor::None),
        Level::Block
    );
    let mut invalid = DefinitionModule::new("native");
    invalid.functions.push(FunctionDef::new(
        FunctionId::new("native", "bad"),
        Accepts::Nothing,
        ReturnRule::BlockIfTrue("missing".into()),
    ));
    assert!(
        Registry::new()
            .register(invalid)
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("missing boolean parameter"))
    );
    let mut structural = DefinitionModule::new("native");
    structural.functions.push(FunctionDef::new(
        FunctionId::new("native", "rows"),
        Accepts::Rows,
        ReturnRule::Fixed(Level::Block),
    ));
    assert!(
        Registry::new()
            .register(structural.clone())
            .unwrap_err()
            .iter()
            .any(|d| d.message.contains("notist::row"))
    );
    assert!(builtins::registry().clone().register(structural).is_ok());
}

#[test]
fn qualified_builtin_calls_produce_the_same_ir_as_prelude_calls() {
    fn normalize(mut item: notist::Item) -> notist::Item {
        item.span = span();
        item.children = item.children.into_iter().map(normalize).collect();
        item
    }
    for (plain, qualified) in [
        ("#callout[note]", "#notist::callout[note]"),
        (
            "#raw(\"code\", block: true)",
            "#notist::raw(\"code\", block: true)",
        ),
        ("#list[#item[first]]", "#notist::list[#notist::item[first]]"),
    ] {
        let (plain_tree, plain_diags) = notist::analyze(plain);
        let (qualified_tree, qualified_diags) = notist::analyze(qualified);
        assert!(plain_diags.is_empty());
        assert!(qualified_diags.is_empty());
        assert_eq!(normalize(plain_tree), normalize(qualified_tree));
    }
}

#[test]
fn multiline_default_strings_share_framing_escapes_and_raw_behavior() {
    for newline in ["\n", "\r\n", "\r"] {
        for (literal, expected) in [
            (
                format!("\"\"\"{newline}  hello\\nworld{newline}\"\"\""),
                "  hello\nworld",
            ),
            (
                format!("r#\"\"\"{newline}  hello\\nworld{newline}\"\"\"#"),
                "  hello\\nworld",
            ),
            (
                format!("\"\"\"{newline}escaped \\\"\"\" stays{newline}\"\"\""),
                "escaped \"\"\" stays",
            ),
        ] {
            let module = analyze_module(
                "pkg",
                &format!("fn f(label: String = {literal}) -> Content;"),
            )
            .unwrap();
            assert_eq!(
                module.functions[0].parameters[0].mode,
                ParameterMode::Default(Value::Str(expected.into()))
            );
            let (document, diagnostics) = notist::analyze(&format!("#text(text: {literal})"));
            assert!(diagnostics.is_empty(), "{literal:?}: {diagnostics:?}");
            assert_eq!(
                document.children[0].children[0].fields.get("text"),
                Some(&Value::Str(expected.into()))
            );
        }
    }
    assert!(
        analyze_module(
            "pkg",
            "fn f(label: String = \"\"\"no framing\"\"\") -> Content;"
        )
        .unwrap_err()
        .iter()
        .any(|d| d.message.contains("must start with a newline"))
    );
}
