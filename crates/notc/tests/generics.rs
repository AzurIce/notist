use notc::{BinaryOp, ProjectDiagnosticKind, Type, TypeError, Value, check, check_project, parse};

#[test]
fn checks_uncalled_generic_bodies_and_every_match_arm_at_definition_site() {
    let source = r#"
fn main() -> Int { 42 }

// Uncalled generic with invalid op on abstract T:
fn bad_abstract<T: Type>(x: T) -> T {
    x + x
}

// Uncalled generic where `match T` arm has a type error:
fn bad_arm<T: Type>(x: T) -> T
where T == Int || T == Float {
    match T {
        Int   => x + x,
        Float => true,
        _     => compile_error("unreachable"),
    }
}
"#;

    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let errors = check(&parsed.program).unwrap_err();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0].declaration, "bad_abstract");
    assert_eq!(
        errors[0].error,
        TypeError::UnsupportedBinaryOp {
            op: BinaryOp::Add,
            left: Type::Param("T".into()),
            right: Type::Param("T".into()),
        }
    );
    assert_eq!(errors[1].declaration, "bad_arm");
    assert_eq!(
        errors[1].error,
        TypeError::Mismatch {
            expected: Type::Float,
            actual: Type::Bool,
        }
    );
}

#[test]
fn enforces_where_guard_at_call_site_and_rejects_uninstantiated_generic_values() {
    let source = r#"
fn identity<T: Type>(x: T) -> T { x }

fn double<T: Type>(x: T) -> T
where T == Int || T == Float {
    match T {
        Int   => x + x,
        Float => x + x,
        _     => compile_error("unreachable"),
    }
}

fn bad_where() -> Bool {
    double(true)
}

fn bad_bare_value() -> Int {
    let f = identity;
    0
}

fn bad_mismatched_pair<T: Type>(a: T, b: T) -> T { a }

fn call_mismatched() -> Int {
    bad_mismatched_pair(1, true)
}
"#;

    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let errors = check(&parsed.program).unwrap_err();
    assert_eq!(errors.len(), 3);
    assert_eq!(errors[0].declaration, "bad_where");
    assert_eq!(
        errors[0].error,
        TypeError::WhereUnsatisfied {
            function: "double".into(),
        }
    );
    assert_eq!(errors[1].declaration, "bad_bare_value");
    assert_eq!(
        errors[1].error,
        TypeError::TypeArgsRequired {
            function: "identity".into(),
            expected: 1,
        }
    );
    assert_eq!(errors[2].declaration, "call_mismatched");
    assert_eq!(
        errors[2].error,
        TypeError::Mismatch {
            expected: Type::Int,
            actual: Type::Bool,
        }
    );
}

#[test]
fn catches_compile_error_reached_by_specialization_even_inside_runtime_branch() {
    let source = r#"
fn sneaky<T: Type>(flag: Bool, x: T) -> Int {
    if flag {
        0
    } else {
        match T {
            Int => x + 1,
            _   => compile_error("sneaky only handles Int"),
        }
    }
}

fn main() -> Int {
    // Even though `flag` is true at runtime, specializing `sneaky<Bool>`
    // folds `match Bool` into the `_ => compile_error(...)` arm at check time.
    sneaky(true, false)
}
"#;

    let parsed = parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let errors = check(&parsed.program).unwrap_err();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].declaration, "sneaky");
    assert_eq!(
        errors[0].error,
        TypeError::CompileError("sneaky only handles Int".into())
    );
}

#[test]
fn supports_cross_module_generics_first_class_instances_and_reachability_lowering() {
    let project = check_project(&[
        (
            "ops.notc",
            r#"
fn identity<T: Type>(x: T) -> T { x }

fn double<T: Type>(x: T) -> T
where T == Int || T == Float || T == String {
    match T {
        Int    => x + x,
        Float  => x + x,
        String => x + x,
        _      => compile_error("unreachable"),
    }
}

fn dead_generic<T: Type>(x: T) -> T { x }
"#,
        ),
        (
            "main.notc",
            r#"
import "./ops.notc" as ops;

fn apply(f: fn(Int) -> Int, x: Int) -> Int {
    f(x)
}

fn main() -> Int {
    let d = ops.double<Int>;
    let x = apply(d, 20);
    let s = ops.double("ok");
    if s == "okok" {
        ops.identity(x + 2)
    } else {
        0
    }
}
"#,
        ),
    ])
    .expect("cross-module generics should check");

    assert!(matches!(project.run("main.notc", "main"), Ok(Value::Int(42))));
    // Lowered instances: main, apply, double<Int>, double<String>, identity<Int> = 5
    // (`dead_generic` and `double<Float>` are not lowered).
    let lowered = project
        .lower_entry("main.notc", "main")
        .expect("main should lower");
    assert_eq!(lowered.function_count(), 5);
}

#[test]
fn rejects_cross_module_where_violation_at_call_site() {
    let errors = check_project(&[
        (
            "ops.notc",
            r#"
fn only_int<T: Type>(x: T) -> T
where T == Int {
    x
}
"#,
        ),
        (
            "main.notc",
            r#"
import "./ops.notc": only_int;
fn main() -> Bool { only_int(true) }
"#,
        ),
    ])
    .unwrap_err();

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].module, "main.notc");
    assert!(matches!(
        errors[0].kind,
        ProjectDiagnosticKind::Type {
            ref declaration,
            error: TypeError::WhereUnsatisfied { ref function },
            ..
        } if declaration == "main" && function == "only_int"
    ));
}
