use notc::{Type, TypeError, Value, check, check_project, parse};

#[test]
fn multi_file_example_runs_main() {
    let project = check_project(&[
        (
            "main.not",
            include_str!("../examples/programs/multi/main.not"),
        ),
        (
            "math.not",
            include_str!("../examples/programs/multi/math.not"),
        ),
    ])
    .expect("multi-file example should check");
    assert!(matches!(project.run("main.not", "main"), Ok(Value::Int(42))));
}

#[test]
fn content_example_runs_all_entries() {
    let source = include_str!("../examples/programs/content.not");
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("content.not should check");
    assert!(matches!(
        program.run("summary"),
        Ok(Value::String(ref s)) if s.as_ref() == "h1: Welcome to ⚡ Notist | h2: Architecture"
    ));
    let Value::Block(doc) = program.run("main").expect("main should evaluate") else {
        panic!("expected Block");
    };
    assert!(doc.plain_text().contains("Welcome to ⚡ Notist"));
    assert!(doc.plain_text().contains("Remember to run sectionize() explicitly."));
}

#[test]
fn generics_example_runs_main() {
    let source = include_str!("../examples/programs/generics.not");
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("generics.not should check");
    assert!(matches!(program.run("main"), Ok(Value::Int(42))));
}

#[test]
fn scalars_example_runs_all_entries() {
    let source = include_str!("../examples/programs/scalars.not");
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("scalars.not should check");
    assert!(matches!(program.run("main"), Ok(Value::Int(42))));
    assert!(matches!(
        program.run("greeting"),
        Ok(Value::String(ref s)) if s.as_ref() == "Hello\n\"Notist\"!"
    ));
}

#[test]
fn higher_order_example_runs_all_entries() {
    let source = include_str!("../examples/programs/higher_order.not");
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("higher_order.not should check");
    assert!(matches!(program.run("main"), Ok(Value::Int(42))));
    assert!(matches!(program.run("shadow_demo"), Ok(Value::Int(42))));
}

#[test]
fn methods_example_runs_main() {
    let source = include_str!("../examples/programs/methods.not");
    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = check(&parsed.program).expect("methods.not should check");
    assert!(matches!(program.run("main"), Ok(Value::Int(42))));
}

#[test]
fn diagnostics_example_recovers_and_reports_uncalled_type_errors() {
    let source = include_str!("../examples/programs/diagnostics.not");
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
