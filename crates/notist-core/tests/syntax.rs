use notist_core::{TypeError, Value, check, parse};

fn run(source: &str, entry: &str) -> Value {
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("source should check");
    program.run(entry).expect("entry should evaluate")
}

#[test]
fn else_if_chains_and_string_methods() {
    let source = r##"
fn classify(s: String) -> String {
    if s.starts_with("#") {
        "tag"
    } else if s.ends_with("!") {
        "shout"
    } else if s.contains("x") {
        "x"
    } else {
        "plain"
    }
}

fn main() -> String {
    let replaced = "banana".replace("na", "NA");
    classify("#notist")
        + ","
        + classify("hi!")
        + ","
        + classify("max")
        + ","
        + classify("  plain  ".trim())
        + ","
        + replaced
}
"##;
    assert!(matches!(
        run(source, "main"),
        Value::String(value) if value.as_ref() == "tag,shout,x,plain,baNANA"
    ));
}

#[test]
fn defaults_fill_trailing_parameters_and_may_reference_earlier_ones() {
    let source = r#"
fn wrap(text: String, left: String = "[", right: String = "]") -> String {
    left + text + right
}

fn open_tag(name: String, prefix: String = "<" + name) -> String {
    prefix + ">"
}

fn answer(n: Int = 42) -> Int {
    n
}

fn main() -> String {
    wrap("x") + "|" + wrap("y", "<") + "|" + wrap("z", "<", ">") + "|" + open_tag("a")
}
"#;
    assert!(matches!(
        run(source, "main"),
        Value::String(value) if value.as_ref() == "[x]|<y]|<z>|<a>"
    ));
    assert!(matches!(run(source, "answer"), Value::Int(42)));
}

#[test]
fn generic_functions_support_value_defaults() {
    let source = r#"
fn keep<T: Type>(value: T, fallback: T = value) -> T {
    value
}

fn inferred() -> String {
    keep("ok")
}

fn explicit() -> String {
    keep<String>("ok")
}
"#;
    for entry in ["inferred", "explicit"] {
        assert!(matches!(
            run(source, entry),
            Value::String(value) if value.as_ref() == "ok"
        ));
    }
}

#[test]
fn method_parameters_can_have_defaults() {
    let source = r#"
fn Int.plus(self, n: Int = 1) -> Int {
    self + n
}

fn main() -> Int {
    41.plus()
}
"#;
    assert!(matches!(run(source, "main"), Value::Int(42)));
}

#[test]
fn content_parameters_can_default_to_empty_content() {
    let source = r#"
fn panel(title: String = "", body: Block = [ ]) -> Block {
    callout("note")[
        #body
    ]
}

fn main() -> Block {
    panel()
}
"#;
    assert!(matches!(run(source, "main"), Value::Block(_)));
}

#[test]
fn external_content_declarations_support_defaults() {
    let source = r#"
fn badge(label: String, count: Int = 0) -> Inline;

fn main() -> Inline {
    badge("new")
}
"#;
    assert!(matches!(run(source, "main"), Value::Inline(_)));
}

#[test]
fn reports_arity_ranges_for_functions_with_defaults() {
    let source = r#"
fn f(a: Int, b: Int = 0) -> Int {
    a + b
}

fn too_few() -> Int {
    f()
}

fn too_many() -> Int {
    f(1, 2, 3)
}
"#;
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let errors = check(&parsed.program).unwrap_err();
    assert_eq!(errors.len(), 2);
    assert_eq!(
        errors[0].error,
        TypeError::ArityRange {
            required: 1,
            total: 2,
            actual: 0,
        }
    );
    assert_eq!(
        errors[1].error,
        TypeError::ArityRange {
            required: 1,
            total: 2,
            actual: 3,
        }
    );
}

#[test]
fn rejects_required_parameters_after_a_default() {
    let parsed = parse("fn bad(a: Int = 0, b: Int) -> Int { a + b }");
    assert!(
        parsed.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("cannot follow a parameter with a default")),
        "{:?}",
        parsed.diagnostics
    );
}
