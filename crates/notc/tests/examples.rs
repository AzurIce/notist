use notc::{Type, TypeError, Value, check, parse};

#[test]
fn higher_order_example_runs_all_entries() {
    let source = include_str!("../examples/programs/higher_order.notc");
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("higher_order.notc should check");
    assert!(matches!(program.run("main"), Ok(Value::Int(42))));
    assert!(matches!(program.run("shadow_demo"), Ok(Value::Int(42))));
}

#[test]
fn methods_example_runs_main() {
    let source = include_str!("../examples/programs/methods.notc");
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("methods.notc should check");
    assert!(matches!(program.run("main"), Ok(Value::Int(42))));
}

#[test]
fn diagnostics_example_recovers_and_reports_uncalled_type_errors() {
    let source = include_str!("../examples/programs/diagnostics.notc");
    let parsed = parse(source);
    assert_eq!(parsed.diagnostics.len(), 1);
    // `healthy`, `unused_type_error`, `Int.add_one`, and `unused_method_error` all survived recovery.
    assert_eq!(parsed.program.root.declarations.len(), 4);

    let errors = check(&parsed.program).unwrap_err();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].declaration, "unused_type_error");
    assert_eq!(
        errors[0].error,
        TypeError::Mismatch {
            expected: Type::Int,
            actual: Type::Bool,
        }
    );
    assert_eq!(errors[1].declaration, "unused_method_error");
    assert_eq!(
        errors[1].error,
        TypeError::UnknownMethod {
            receiver: Type::Bool,
            method: "add_one".into(),
        }
    );
}
