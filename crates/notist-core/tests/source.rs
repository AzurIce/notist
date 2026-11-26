use notist_core::{BinaryOp, ExecutionError, Type, TypeError, UnaryOp, Value, check, parse};

fn slice(source: &str, span: notist_core::Span) -> &str {
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

#[test]
fn evaluates_all_scalars_string_forms_precedence_and_short_circuit() {
    let source = r###"
fn recurse() -> Bool {
    recurse()
}

fn Int.inc(self) -> Int {
    self + 1
}

fn Float.half(self) -> Float {
    self / 2.0
}

fn empty_unit() -> Unit {}

fn seq_unit() -> Unit {
    let x = 1;
    x + 2;
}

fn main() -> Bool {
    let u1 = ();
    let u2 = empty_unit();
    let u3 = seq_unit();
    let unit_ok = (u1 == u2) && !(u2 != u3);

    // Operator precedence & method calls on Int and Float literals
    let math_ok = 1 + 2 * 3 - 4 / 2 + 10 % 4 == 7
        && -3 * -2 == 6
        && 41.inc() == 42
        && 5.0.half() == 2.5
        && -9223372036854775808 < 0;

    // Short-circuit logic never enters `recurse()`
    let logic_ok = !(false && recurse()) && (true || recurse());

    // All 4 string literal forms + concatenation + lexicographical ordering
    let s1 = "a\nb\t\"c\"\\";
    let s2 = """
a
b	"c"\\
""";
    let s3 = r#"a\n"#;
    let s4 = r##"""
raw "# text
"""##;
    let str_ok = s1 == s2 && s3 != "a\n" && s4 == "raw \"# text" && ("ab" + "c" > "aba");

    unit_ok && math_ok && logic_ok && str_ok
}
"###;

    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("scalars program should check");
    assert!(matches!(program.run("main"), Ok(Value::Bool(true))));
}

#[test]
fn catches_checked_integer_runtime_errors() {
    let source = r#"
fn div_zero() -> Int { 10 / 0 }
fn rem_zero() -> Int { 10 % 0 }
fn min_neg() -> Int {
    let m = -9223372036854775808;
    -m
}
fn min_div_neg1() -> Int {
    let m = -9223372036854775808;
    m / -1
}
fn mul_overflow() -> Int { 9223372036854775807 * 2 }
"#;

    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("should check");
    assert_eq!(program.run("div_zero").unwrap_err(), ExecutionError::DivisionByZero);
    assert_eq!(program.run("rem_zero").unwrap_err(), ExecutionError::DivisionByZero);
    assert_eq!(program.run("min_neg").unwrap_err(), ExecutionError::IntegerOverflow);
    assert_eq!(program.run("min_div_neg1").unwrap_err(), ExecutionError::IntegerOverflow);
    assert_eq!(program.run("mul_overflow").unwrap_err(), ExecutionError::IntegerOverflow);
}

#[test]
fn rejects_ill_typed_scalar_and_operator_expressions() {
    let source = r#"
fn id(x: Int) -> Int { x }
fn bad_neg() -> Bool { -true }
fn bad_fn_eq() -> Bool { id == id }
fn bad_bool_lt() -> Bool { true < false }
fn bad_str_sub() -> String { "a" - "b" }
fn bad_mixed_add() -> Int { 1 + 1.0 }
fn bad_not_int() -> Bool { !1 }
"#;

    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let errors = check(&parsed.program).unwrap_err();
    assert_eq!(errors.len(), 6);
    assert_eq!(
        errors[0].error,
        TypeError::UnsupportedUnaryOp {
            op: UnaryOp::Neg,
            operand: Type::Bool,
        }
    );
    assert_eq!(
        errors[1].error,
        TypeError::UnsupportedBinaryOp {
            op: BinaryOp::Eq,
            left: Type::Function(vec![Type::Int], Box::new(Type::Int)),
            right: Type::Function(vec![Type::Int], Box::new(Type::Int)),
        }
    );
    assert_eq!(
        errors[2].error,
        TypeError::UnsupportedBinaryOp {
            op: BinaryOp::Lt,
            left: Type::Bool,
            right: Type::Bool,
        }
    );
    assert_eq!(
        errors[3].error,
        TypeError::UnsupportedBinaryOp {
            op: BinaryOp::Sub,
            left: Type::String,
            right: Type::String,
        }
    );
    assert_eq!(
        errors[4].error,
        TypeError::Mismatch {
            expected: Type::Int,
            actual: Type::Float,
        }
    );
    assert_eq!(
        errors[5].error,
        TypeError::Mismatch {
            expected: Type::Bool,
            actual: Type::Int,
        }
    );
}
