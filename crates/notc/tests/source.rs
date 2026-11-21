use notc::{Type, TypeError, Value, check, parse};

fn slice(source: &str, span: notc::Span) -> &str {
    &source[span.start as usize..span.end as usize]
}

#[test]
fn stage1_acceptance_calls_function_value_through_local_binding() {
    let source = r#"
// Stage 1 acceptance: host picks an entry, binds a function value locally, and gets Int.
fn inc(x: Int) -> Int {
    x + 1
}

/* nested /* block */ comment */
fn identity(f: fn(Int) -> Int) -> fn(Int) -> Int {
    f
}

fn entry() -> Int {
    let step = identity(inc);
    let base = 40 + 1;
    step(base)
}
"#;

    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("source should pass static checking");
    assert!(matches!(program.run("entry"), Ok(Value::Int(42))));
}

#[test]
fn uncalled_function_type_error_fails_with_exact_source_span() {
    let source = r#"
fn main() -> Int {
    42
}

fn unused(flag: Bool) -> Int {
    if flag {
        1
    } else {
        false
    }
}
"#;

    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let errors = check(&parsed.program).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].declaration, "unused");
    assert_eq!(
        errors[0].error,
        TypeError::Mismatch {
            expected: Type::Int,
            actual: Type::Bool,
        }
    );
    assert_eq!(slice(source, errors[0].span), "false");
}

#[test]
fn parses_and_runs_methods_else_if_and_function_receiver_methods() {
    let source = r#"
fn inc(x: Int) -> Int {
    x + 1
}

fn Int.add_to(self, delta: Int,) -> Int {
    self + delta
}

fn (fn(Int) -> Int).twice(self, x: Int) -> Int {
    self(self(x))
}

fn main() -> Int {
    let base = if false {
        0
    } else if true {
        39
    } else {
        100
    };
    let f = inc;
    f.twice(base).add_to(1)
}
"#;

    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("source should check");
    assert!(matches!(program.run("main"), Ok(Value::Int(42))));
}

#[test]
fn syntax_errors_do_not_swallow_following_independent_declarations() {
    let source = r#"
fn broken_header(x: Int -> Int {
    x + 1
}

fn healthy_one() -> Int {
    10
}

fn unclosed_body() -> Int {
    let x = 1 +

fn Int.plus_one(self) -> Int {
    self + 1
}

fn healthy_two() -> Int {
    healthy_one().plus_one() + 31
}
"#;

    let parsed = parse(source);
    assert!(
        parsed.diagnostics.len() >= 2,
        "expected diagnostics for both broken declarations, got {:?}",
        parsed.diagnostics
    );
    // All three healthy declarations survived recovery and still type-check & run.
    assert_eq!(parsed.program.root.declarations.len(), 3);
    let checked = check(&parsed.program).expect("recovered healthy declarations should check");
    assert!(matches!(checked.run("healthy_two"), Ok(Value::Int(42))));
}

#[test]
fn reports_exact_spans_for_lex_errors_and_integer_overflow() {
    let source = r#"
fn bad_tokens() -> Int {
    @ 99999999999999999999999999
}

fn ok() -> Int {
    1_000 + 24
}
/* unterminated
"#;

    let parsed = parse(source);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| slice(source, d.span) == "@" && d.message.contains("unexpected character")),
        "{:?}",
        parsed.diagnostics
    );
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.message.contains("overflows `Int`")
                && slice(source, d.span) == "99999999999999999999999999"),
        "{:?}",
        parsed.diagnostics
    );
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.message.contains("unterminated block comment")
                && slice(source, d.span).starts_with("/* unterminated")),
        "{:?}",
        parsed.diagnostics
    );

    let checked = check(&parsed.program).expect("recovered `ok` should check");
    assert!(matches!(checked.run("ok"), Ok(Value::Int(1024))));
}

#[test]
fn caps_deep_syntax_nesting_without_stack_overflow() {
    let open = "(".repeat(256);
    let close = ")".repeat(256);
    let source = format!("fn deep() -> Int {{ {open}1{close} }}");

    let parsed = parse(&source);
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|d| d.message.contains("nesting depth")),
        "{:?}",
        parsed.diagnostics
    );
}
