use notist::{Context, Diagnostic, FunctionId, Item, Pipeline, Type, Value};

fn context(module: &str) -> Context {
    Context::default()
        .with_module(
            Pipeline::default()
                .compile_module("content.not", "result", "result/lib.notc", module)
                .unwrap(),
        )
        .unwrap()
}

fn diagnostics(module: &str) -> Vec<Diagnostic> {
    let pipeline = Pipeline::default();
    let module = match pipeline.compile_module("content.not", "result", "result/lib.notc", module) {
        Ok(module) => module,
        Err(diagnostics) => return diagnostics,
    };
    let context = match Context::default().with_module(module) {
        Ok(context) => context,
        Err(diagnostics) => return diagnostics,
    };
    pipeline.check_module("result", &context)
}

fn text(root: &Item) -> String {
    root.descendants()
        .filter_map(|item| match item.field("text") {
            Some(Value::Str(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}

fn evaluate(module: &str, call: &str) -> notist::Located<Item> {
    let context = context(module);
    let pipeline = Pipeline::default();
    let errors = pipeline.check_module("result", &context);
    assert!(errors.is_empty(), "{module}: {errors:?}");
    let analysis = pipeline.analyze("doc.not", call, &context).unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{call}: {:?}",
        analysis.diagnostics()
    );
    let output = pipeline.evaluate(&analysis, &context);
    assert!(
        output.diagnostics.is_empty(),
        "{call}: {:?}",
        output.diagnostics
    );
    notist::Located::new(output.root, output.sources)
}

fn assert_error(module: &str, needle: &str) {
    let errors = diagnostics(module);
    assert!(
        errors.iter().any(|d| d.message.contains(needle)),
        "{module}: {errors:?}"
    );
}

#[test]
fn results_construct_match_and_return_errors_as_ordinary_values() {
    let module = r#"
fn choose(enabled: Bool, value: String) -> Result<String, String> {
    if enabled { ok(value) } else { err("unavailable") }
}
fn show(enabled: Bool, value: String) -> InlineContent {
    [#text(text: match choose(enabled, value) {
        ok(value) => value + "!",
        err(value) => value + "?",
    })]
}
"#;
    assert_eq!(
        text(&evaluate(module, "#self.show(true, \"hello\")")),
        "hello!"
    );
    // The payload binding shadows the outer `value` even across lazy content.
    assert_eq!(
        text(&evaluate(module, "#self.show(false, \"hello\")")),
        "unavailable?"
    );
}

#[test]
fn constructors_take_types_from_returns_arguments_and_typed_siblings() {
    let module = r#"
fn accept(value: Result<Int, String>) -> String {
    match value { ok(n) => "number", err(e) => e }
}
fn explicit() -> String {
    let value = err<Int, String>("explicit");
    accept(value)
}
fn sibling(flag: Bool) -> String {
    let value = if flag { ok<Int, String>(1) } else { err("sibling") };
    accept(value)
}
fn swapped(flag: Bool) -> String {
    let value = if flag { err("swapped") } else { ok<Int, String>(1) };
    accept(value)
}
fn show(flag: Bool) -> InlineContent {
    [#text(text: accept(ok(1)) + accept(err("argument")) + explicit() + sibling(flag) + swapped(flag))]
}
"#;
    assert_eq!(
        text(&evaluate(module, "#self.show(true)")),
        "numberargumentexplicitnumberswapped"
    );
    assert_eq!(
        text(&evaluate(module, "#self.show(false)")),
        "numberargumentexplicitsiblingnumber"
    );
}

#[test]
fn result_match_bodies_take_the_declared_return_type_or_a_typed_sibling() {
    let module = r#"
fn input(flag: Bool) -> Result<Int, String> {
    if flag { ok(1) } else { err("input") }
}
fn mapped(flag: Bool) -> Result<String, Bool> {
    match input(flag) { ok(n) => ok("mapped"), err(e) => err(false) }
}
fn sibling(flag: Bool) -> String {
    let value = match input(flag) {
        ok(n) => ok<String, Bool>("sibling"),
        err(e) => err(false),
    };
    match value { ok(s) => s, err(e) => "error" }
}
fn show(flag: Bool) -> InlineContent {
    let label = match mapped(flag) { ok(s) => s, err(e) => "error" };
    [#text(text: label + sibling(flag))]
}
"#;
    assert_eq!(text(&evaluate(module, "#self.show(true)")), "mappedsibling");
    assert_eq!(text(&evaluate(module, "#self.show(false)")), "errorerror");
}

#[test]
fn option_array_and_result_nest_without_implicit_conversions() {
    let module = r#"
fn nullable() -> Result<Option<String>, Bool> { ok(none) }
fn wrapped() -> Option<Result<Int, String>> { some(err("nested")) }
fn nested() -> Result<String, Result<Int, Bool>> { err(ok(1)) }
fn values(flag: Bool) -> Array<Result<Int, String>> {
    (ok(1), if flag { ok(2) } else { err("array") })
}
fn widget(values: Array<Result<Int, String>>) -> InlineContent;
fn show() -> InlineContent {
    let a = match nullable() {
        ok(v) => match v { some(s) => s, none => "null" },
        err(e) => "unexpected",
    };
    let b = match wrapped() {
        some(v) => match v { ok(n) => "number", err(e) => e },
        none => "missing",
    };
    let c = match nested() {
        ok(s) => s,
        err(e) => match e { ok(n) => "inner", err(b) => "other" },
    };
    [#text(text: a + b + c) #widget(values(false))]
}
"#;
    let root = evaluate(module, "#self.show()");
    assert_eq!(text(&root).trim_end(), "nullnestedinner");
    let widget = root
        .find(|item| item.function_id() == Some(FunctionId::new("result", "widget")))
        .unwrap();
    assert_eq!(
        widget.field("values"),
        Some(&Value::Array(
            vec![
                Value::Ok(Box::new(Value::Int(1))),
                Value::Err(Box::new(Value::Str("array".into()))),
            ]
            .into()
        ))
    );
}

#[test]
fn result_type_expressions_and_generic_inference_keep_both_parameters() {
    let module = r#"
fn identity<T: Type>(value: T) -> T { value }
fn relay<T: Type, E: Type>(value: Result<T, E>) -> Result<T, E> { value }
fn make<T: Type, E: Type>(value: T) -> Result<T, E> { ok(value) }
fn show() -> InlineContent {
    let value = relay(identity<Result<Int, String>>(ok(1)));
    let other = make<String, Bool>("generic");
    let wrapped = identity<Option<Result<Int, String>>>(some(err("wrapped")));
    let a = match value { ok(n) => "number", err(e) => e };
    let b = match other { ok(s) => s, err(b) => "error" };
    let c = match wrapped {
        some(v) => match v { ok(n) => "number", err(e) => e },
        none => "empty",
    };
    [#text(text: a + b + c)]
}
"#;
    assert_eq!(
        text(&evaluate(module, "#self.show()")),
        "numbergenericwrapped"
    );
}

#[test]
fn compile_time_results_prune_bodies_without_losing_the_nominal_type() {
    let module = r#"
fn made() -> Result<Int, String> { ok(1) }
fn keep<V: Result<Int, String>>() -> Result<Int, String> { V }
fn show() -> InlineContent {
    let a = match comptime made() {
        ok(n) => "constant",
        err(e) => compile_error("unselected body"),
    };
    let b = match err<Int, String>("error") {
        ok(n) => compile_error("unselected body"),
        err(e) => e,
    };
    let c = match keep<ok(1)>() { ok(n) => "comptime parameter", err(e) => e };
    let d = match some(ok<Int, String>(1)) {
        some(v) => match v { ok(n) => "nested", err(e) => e },
        none => "none",
    };
    [#text(text: a + b + c + d)]
}
"#;
    assert_eq!(
        text(&evaluate(module, "#self.show()")),
        "constanterrorcomptime parameternested"
    );
}

#[test]
fn results_support_variant_and_payload_equality_but_not_ordering() {
    let module = r#"
fn same(a: Result<Int, String>, b: Result<Int, String>) -> String {
    if a == b { "same" } else { "different" }
}
fn show() -> InlineContent {
    [#text(text: same(ok(1), ok(1)) + same(ok(1), ok(2)) + same(err("a"), err("a")) + same(ok(1), err("a")))]
}
"#;
    assert_eq!(
        text(&evaluate(module, "#self.show()")),
        "samedifferentsamedifferent"
    );
    assert_error(
        "fn bad(a: Result<Int, String>, b: Result<Int, String>) -> Bool { a < b }",
        "not defined",
    );
    assert_error(
        "fn bad() -> Bool { ok<Int, String>(1) == ok<Int, Bool>(1) }",
        "equal operand types",
    );
}

#[test]
fn missing_constructor_types_require_context_or_explicit_arguments() {
    for module in [
        "fn bad() -> String { let value = ok(1); \"x\" }",
        "fn bad() -> String { let value = err(\"bad\"); \"x\" }",
        "fn bad(flag: Bool) -> String { let value = if flag { ok(1) } else { err(\"bad\") }; \"x\" }",
    ] {
        assert_error(module, "cannot determine");
    }
}

#[test]
fn constructor_and_type_arity_and_payload_types_are_checked() {
    for (module, needle) in [
        (
            "fn bad() -> Result<Int, String> { ok() }",
            "exactly one positional",
        ),
        (
            "fn bad() -> Result<Int, String> { ok(1, 2) }",
            "exactly one positional",
        ),
        (
            "fn bad() -> Result<Int, String> { err(value: \"x\") }",
            "exactly one positional",
        ),
        (
            "fn bad() -> Result<Int, String> { ok<Int>(1) }",
            "2 positional type",
        ),
        (
            "fn bad() -> Result<Int, String> { ok<1, String>(1) }",
            "known data types",
        ),
        (
            "fn bad() -> Result<Int, String> { ok(\"wrong\") }",
            "expected integer",
        ),
        (
            "fn bad() -> Result<Int, String> { err(1) }",
            "expected string",
        ),
        (
            "fn bad() -> Result<Int, String> { ok(1)[body] }",
            "does not take content",
        ),
        ("fn bad() -> Result<Int> { ok(1) }", "2 type arguments"),
        ("fn bad() -> Result { ok(1) }", "2 type arguments"),
        (
            "fn bad() -> Result<Int, String, Bool> { ok(1) }",
            "2 type arguments",
        ),
    ] {
        assert_error(module, needle);
    }
}

#[test]
fn results_cannot_wrap_types_or_content_in_either_variant() {
    for module in [
        "fn bad() -> Result<InlineContent, String> { ok([body]) }",
        "fn bad() -> Result<Int, BlockContent> { err([ body ]) }",
        "fn bad() -> Result<Int, Type> { ok(1) }",
        "fn bad() -> Array<Result<Int, InlineContent>> { (,) }",
    ] {
        assert_error(module, "runtime signatures");
    }
    for module in [
        "fn bad() -> String { let value = ok<Type, String>(Int); \"x\" }",
        "fn bad() -> String { let value = err<Int, InlineContent>([body]); \"x\" }",
        "fn bad() -> String { let value = Result<Int, BlockContent>; \"x\" }",
    ] {
        assert_error(module, "known data types");
    }
}

#[test]
fn inactive_payload_types_cannot_be_changed_by_constant_folding_or_containers() {
    for module in [
        "fn bad() -> Result<Int, Bool> { ok<Int, String>(1) }",
        "fn bad() -> Result<Bool, String> { err<Int, String>(\"x\") }",
        "fn bad() -> Result<Int, Bool> { comptime ok<Int, String>(1) }",
        "fn original() -> Result<Int, String> { ok(1) } fn bad() -> Result<Int, Bool> { comptime original() }",
        "fn bad() -> Array<Result<Int, Bool>> { (ok<Int, String>(1),) }",
        "fn bad() -> Option<Result<Int, Bool>> { some(ok<Int, String>(1)) }",
        "fn bad() -> Result<Int, Bool> { match some(ok<Int, String>(1)) { some(v) => v, none => ok(0) } }",
    ] {
        assert_error(module, "expected");
    }
    assert_error(
        r#"
fn accept(value: Result<Int, Bool>) -> String { "x" }
fn bad() -> String { accept(ok<Int, String>(1)) }
"#,
        "must be Result",
    );
    assert_error(
        r#"
fn accept<V: Result<Int, Bool>>() -> String { "x" }
fn bad() -> String { accept<ok<Int, String>(1)>() }
"#,
        "expected Result",
    );
}

#[test]
fn result_matches_require_exhaustive_well_formed_reachable_patterns() {
    for (arms, needle) in [
        ("ok(n) => \"ok\"", "requires both"),
        ("err(e) => e", "requires both"),
        ("ok(1) => \"ok\", _ => \"other\"", "Result patterns"),
        ("some(n) => \"ok\", _ => \"other\"", "Result patterns"),
        (
            "ok<Int, String>(n) => \"ok\", _ => \"other\"",
            "Result patterns",
        ),
        (
            "ok(n) => \"ok\", ok(m) => \"again\", err(e) => e",
            "unreachable",
        ),
        ("_ => \"other\", ok(n) => \"ok\"", "unreachable"),
        (
            "ok(n) => 1, err(e) => \"error\"",
            "branches return different types",
        ),
    ] {
        assert_error(
            &format!("fn bad(value: Result<Int, String>) -> String {{ match value {{ {arms} }} }}"),
            needle,
        );
    }
    // Coverage is checked even when the selected variant is already known.
    assert_error(
        "fn bad() -> String { match ok<Int, String>(1) { ok(n) => \"ok\" } }",
        "requires both",
    );
    let module = r#"
fn show(value: Bool) -> InlineContent {
    let result = if value { ok<Int, String>(1) } else { err("error") };
    let label = match result { ok(_) => "ok", _ => "fallback" };
    [#text(text: label)]
}
"#;
    assert_eq!(text(&evaluate(module, "#self.show(true)")), "ok");
    assert_eq!(text(&evaluate(module, "#self.show(false)")), "fallback");
}

#[test]
fn arm_bindings_do_not_escape_or_replace_the_subject_type() {
    assert_error(
        r#"
fn bad(value: Result<Int, String>) -> String {
    let label = match value { ok(n) => "ok", err(e) => e };
    e
}
"#,
        "unbound reference `e`",
    );
    assert_error(
        r#"
fn accept(value: Int) -> String { "x" }
fn bad(value: Result<Int, String>) -> String {
    match value { ok(n) => accept(value), err(e) => e }
}
"#,
        "must be integer",
    );
}

#[test]
fn result_types_accept_only_the_matching_variant_payloads() {
    let ty = Type::Result(Box::new(Type::Int), Box::new(Type::String));
    assert_eq!(ty.to_string(), "Result<integer, string>");
    assert!(ty.data() && ty.runtime() && ty.concrete());
    assert!(ty.accepts(&Value::Ok(Box::new(Value::Int(1)))));
    assert!(ty.accepts(&Value::Err(Box::new(Value::Str("error".into())))));
    assert!(!ty.accepts(&Value::Ok(Box::new(Value::Str("wrong".into())))));
    assert!(!ty.accepts(&Value::Err(Box::new(Value::Int(1)))));
    assert!(!ty.accepts(&Value::Int(1)));
    assert!(!ty.accepts(&Value::Some(Box::new(Value::Int(1)))));
    assert_ne!(ty, Type::Result(Box::new(Type::Int), Box::new(Type::Bool)));
    for invalid in [Type::Type, Type::InlineContent, Type::BlockContent] {
        for ty in [
            Type::Result(Box::new(invalid.clone()), Box::new(Type::String)),
            Type::Result(Box::new(Type::Int), Box::new(invalid)),
        ] {
            assert!(!ty.data());
            assert!(!ty.runtime());
        }
    }
}

#[test]
fn result_payloads_are_included_in_component_protocol_and_path_checks() {
    use notist_html::components::{encode_parameter, value_json};
    let ok = Value::Ok(Box::new(Value::Int(i64::MAX)));
    let err = Value::Err(Box::new(Value::Some(Box::new(Value::Float(-0.0)))));
    assert_eq!(
        value_json(&ok),
        serde_json::json!(["ok", ["int", "9223372036854775807"]])
    );
    assert_eq!(
        value_json(&err),
        serde_json::json!(["err", ["some", ["float", "8000000000000000"]]])
    );
    assert_eq!(encode_parameter(&ok).unwrap(), value_json(&ok).to_string());
    let path = notist::ResourcePath::resolve("/private", "doc.not", "data.txt").unwrap();
    for value in [
        Value::Ok(Box::new(Value::Path(path.clone()))),
        Value::Err(Box::new(Value::Path(path))),
    ] {
        assert!(value.contains_path());
        assert!(encode_parameter(&value).is_err());
    }
}

#[cfg(feature = "serde")]
#[test]
fn result_serde_and_instance_keys_preserve_variants_and_nested_float_bits() {
    use notist::ComptimeValue;
    for value in [
        Value::Ok(Box::new(Value::Unit)),
        Value::Err(Box::new(Value::None)),
        Value::Ok(Box::new(Value::Err(Box::new(Value::Int(1))))),
        Value::Err(Box::new(Value::Float(f64::from_bits(
            0x7ff8_0000_0000_0042,
        )))),
        Value::Ok(Box::new(Value::Float(-0.0))),
    ] {
        let encoded = serde_json::to_string(&value).unwrap();
        let restored: Value = serde_json::from_str(&encoded).unwrap();
        // NaN is deliberately not compared with PartialEq.
        assert_eq!(
            ComptimeValue::Data(restored).key(),
            ComptimeValue::Data(value).key()
        );
    }
    let payload = Value::Int(1);
    assert_ne!(
        ComptimeValue::Data(Value::Ok(Box::new(payload.clone()))).key(),
        ComptimeValue::Data(Value::Err(Box::new(payload))).key(),
    );
    let ty = Type::Result(
        Box::new(Type::Option(Box::new(Type::Int))),
        Box::new(Type::String),
    );
    assert_eq!(
        serde_json::from_str::<Type>(&serde_json::to_string(&ty).unwrap()).unwrap(),
        ty
    );
}

#[test]
fn matches_read_the_subject_once_and_cannot_catch_host_failures() {
    use notist::{EvaluationInputs, EvaluationOutcome, InputError, InputRead, ResourcePath};
    use std::{cell::Cell, sync::Arc};

    struct Inputs {
        response: InputRead,
        calls: Cell<usize>,
    }
    impl EvaluationInputs for Inputs {
        fn file(&self, _: &ResourcePath) -> InputRead {
            self.calls.set(self.calls.get() + 1);
            self.response.clone()
        }
    }
    let module = r#"
fn read(source: Path) -> Result<String, String> { ok(read_text(source)) }
fn show(source: Path) -> InlineContent {
    match read(source) {
        ok(s) => [#text(text: s + s)],
        err(e) => [fallback],
    }
}
"#;
    let context = context(module).with_resource_root("/project");
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("doc.not", "#self.show(p\"data.txt\")", &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    for response in [
        InputRead::Ready(Arc::from(b"read once".as_slice())),
        InputRead::Failed(InputError::NotFound),
        InputRead::Pending,
    ] {
        let inputs = Inputs {
            response: response.clone(),
            calls: Cell::new(0),
        };
        let result = pipeline.evaluate_with_inputs(&analysis, &context, &inputs);
        assert_eq!(inputs.calls.get(), 1);
        match (response, result) {
            (InputRead::Ready(_), EvaluationOutcome::Complete(output)) => {
                assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
                assert_eq!(text(&output.root), "read onceread once");
                assert_eq!(output.dependencies.len(), 1);
            }
            (InputRead::Failed(_), EvaluationOutcome::Complete(output)) => {
                assert!(
                    output
                        .diagnostics
                        .iter()
                        .any(|d| d.message.contains("resource not found"))
                );
                assert!(!text(&output.root).contains("fallback"));
            }
            (InputRead::Pending, EvaluationOutcome::NeedResources(needed)) => {
                assert_eq!(needed.len(), 1)
            }
            (_, outcome) => panic!("unexpected outcome: {outcome:?}"),
        }
    }
}

#[test]
fn native_component_fields_keep_errors_and_optional_results_distinct() {
    let module = r#"
fn widget(value: Result<Int, String>, optional?: Result<Int, String>) -> InlineContent;
fn show() -> InlineContent { [#widget(err("<invalid>"), ok(1))#widget(ok(2))] }
"#;
    let context = context(module);
    let definition = context
        .registry()
        .get(&FunctionId::new("result", "widget"))
        .unwrap();
    let mut html = notist_html::HtmlRegistry::default();
    html.bind_component(definition, "https://example.test/widget.js")
        .unwrap();
    let root = evaluate(module, "#self.show()");
    let output = notist_html::Renderer::new()
        .with_registry(html)
        .render_with_diagnostics(&root);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert!(
        output
            .html
            .contains("[&quot;err&quot;,[&quot;string&quot;,&quot;&lt;invalid&gt;&quot;]]"),
        "{}",
        output.html
    );
    let widgets: Vec<_> = root
        .descendants()
        .filter(|item| item.function_id() == Some(definition.id.clone()))
        .collect();
    assert_eq!(
        widgets[0].field("optional"),
        Some(&Value::Ok(Box::new(Value::Int(1))))
    );
    assert!(widgets[1].field("optional").is_none());
    assert_eq!(output.html.matches("notist-optional=").count(), 1);
    let inspection = notist::json::item(widgets[0], &root.sources).unwrap();
    assert_eq!(
        inspection["fields"]["value"],
        serde_json::json!({"err":"<invalid>"})
    );
    assert_eq!(
        inspection["typed_fields"][1][1],
        serde_json::json!(["value", ["err", ["string", "<invalid>"]]])
    );
}

#[test]
fn standalone_markup_cannot_hide_result_construction_in_arguments() {
    let context = context("fn widget(value: Result<Int, String>) -> InlineContent;");
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("doc.not", "#self.widget(ok<Int, String>(1))", &context)
        .unwrap();
    assert!(
        analysis
            .diagnostics()
            .iter()
            .any(|d| d.message.contains("data literals")),
        "{:?}",
        analysis.diagnostics()
    );
}

#[test]
fn option_matches_can_return_results_and_optional_result_arguments_can_branch() {
    let module = r#"
fn converted(value?: String) -> Result<Int, String> {
    match value { some(s) => err(s), none => ok(1) }
}
fn optional(value?: Result<Int, String>) -> String {
    match value {
        some(v) => match v { ok(n) => "ok", err(e) => e },
        none => "missing",
    }
}
fn show(flag: Bool) -> InlineContent {
    let value = match converted() { ok(n) => "converted", err(e) => e };
    [#text(text: value + optional(if flag { ok(1) } else { err("bad") }) + optional(none))]
}
"#;
    assert_eq!(
        text(&evaluate(module, "#self.show(true)")),
        "convertedokmissing"
    );
    assert_eq!(
        text(&evaluate(module, "#self.show(false)")),
        "convertedbadmissing"
    );
}

#[test]
fn result_replacements_normalize_both_parameters_and_preserve_signature_checks() {
    use notist::{MemoryResources, RenderOptions, Vault};

    let mut resources = MemoryResources::new("/project");
    resources.insert(
        "original.notc",
        br#"
fn choose<T: Type, E: Type>(value: T, error: E) -> Result<T, E> { ok(value) }
fn show() -> InlineContent {
    [#text(text: match choose(1, "replaced") { ok(n) => "original", err(e) => e })]
}
"#
        .to_vec(),
    );
    resources.insert(
        "replacement.notc",
        br#"
fn choose<A: Type, B: Type>(value: A, error: B) -> Result<A, B> { err(error) }
"#
        .to_vec(),
    );
    resources.insert(
        "Notist.toml",
        br#"
[[extensions]]
kind = "replace"
imports = { original = "original.notc", replacement = "replacement.notc" }
from = "original.choose"
to = "replacement.choose"
"#
        .to_vec(),
    );
    let output = Vault::new(resources.clone())
        .render_html(
            "doc.not",
            "#import \"original.notc\" as original;\n#original.show()",
            RenderOptions::default(),
        )
        .unwrap();
    assert!(
        output.analysis.diagnostics().is_empty(),
        "{:?}",
        output.analysis.diagnostics()
    );
    assert!(
        output.evaluation.diagnostics.is_empty(),
        "{:?}",
        output.evaluation.diagnostics
    );
    assert_eq!(text(&output.evaluation.root), "replaced");

    resources.insert(
        "replacement.notc",
        br#"
fn choose<A: Type, B: Type>(value: A, error: B) -> Result<A, String> { err("different") }
"#
        .to_vec(),
    );
    let error = Vault::new(resources)
        .render_html(
            "doc.not",
            "#import \"original.notc\" as original;\n#original.show()",
            RenderOptions::default(),
        )
        .unwrap_err();
    let notist::VaultError::Environment(errors) = error else {
        panic!("expected replacement validation")
    };
    assert!(
        errors
            .iter()
            .any(|d| d.diagnostic.message.contains("result types differ")),
        "{errors:?}"
    );
}

#[test]
fn host_supplied_result_defaults_keep_the_declared_inactive_type() {
    let pipeline = Pipeline::default();
    let mut module = pipeline.compile_module(
        "content.not", "result", "result/lib.notc",
        "fn widget(value: Result<Int, String>) -> InlineContent; fn show() -> InlineContent { [#widget()] }",
    ).unwrap();
    std::sync::Arc::make_mut(&mut module.definitions).functions[0].parameters[0].mode =
        notist::ParameterMode::Default(Value::Ok(Box::new(Value::Int(1))));
    let context = Context::default().with_module(module).unwrap();
    let analysis = pipeline
        .analyze("doc.not", "#self.show()", &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let output = pipeline.evaluate(&analysis, &context);
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    let widget = output
        .root
        .find(|item| item.function_id() == Some(FunctionId::new("result", "widget")))
        .unwrap();
    assert_eq!(
        widget.field("value"),
        Some(&Value::Ok(Box::new(Value::Int(1))))
    );
}
