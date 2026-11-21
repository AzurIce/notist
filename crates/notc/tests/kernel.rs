use Type::{Bool, Int};
use notc::{
    CheckedProgram, Decl, Diagnostic, ExecutionError, Expr, Function, Module, Program, Type,
    TypeError, Value, check,
};

fn def<const N: usize>(
    name: &str,
    parameters: [(&str, Type); N],
    returns: Type,
    body: Expr,
) -> Function {
    Function {
        name: name.into(),
        parameters: parameters
            .into_iter()
            .map(|(name, ty)| (name.into(), ty))
            .collect(),
        returns,
        body,
    }
}

fn fun_ty<const N: usize>(parameters: [Type; N], returns: Type) -> Type {
    Type::Function(Vec::from(parameters), Box::new(returns))
}

fn var(name: &str) -> Expr {
    Expr::Name(name.into())
}

fn call<const N: usize>(target: Expr, args: [Expr; N]) -> Expr {
    Expr::Call(Box::new(target), Vec::from(args))
}

fn add(left: Expr, right: Expr) -> Expr {
    Expr::Add(Box::new(left), Box::new(right))
}

fn bind(name: &str, value: Expr, body: Expr) -> Expr {
    Expr::Let(name.into(), Box::new(value), Box::new(body))
}

fn branch(condition: Expr, yes: Expr, no: Expr) -> Expr {
    Expr::If(Box::new(condition), Box::new(yes), Box::new(no))
}

fn source<const N: usize>(functions: [Function; N]) -> Program {
    Program {
        root: Module {
            declarations: functions.into_iter().map(Decl::Function).collect(),
        },
    }
}

fn checked<const N: usize>(functions: [Function; N]) -> CheckedProgram {
    check(&source(functions)).expect("fixture should pass static checking")
}

#[track_caller]
fn assert_int(program: &CheckedProgram, entry: &str, expected: i64) {
    let actual = program.run(entry);
    assert!(
        matches!(actual, Ok(Value::Int(n)) if n == expected),
        "{entry}: expected Int({expected}), got {actual:?}"
    );
}

#[test]
fn passes_returns_and_calls_a_function_value() {
    let unary = fun_ty([Int], Int);
    let returned = call(var("identity"), [var("inc")]);
    let program = checked([
        def("main", [], Int, call(returned, [Expr::Int(41)])),
        def("identity", [("f", unary.clone())], unary, var("f")),
        def("inc", [("x", Int)], Int, add(var("x"), Expr::Int(1))),
    ]);

    assert_int(&program, "main", 42);
}

#[test]
fn local_function_value_can_shadow_a_top_level_function() {
    let program = checked([
        def(
            "main",
            [],
            Int,
            bind("pick", var("answer"), call(var("pick"), [])),
        ),
        def("pick", [], Int, Expr::Int(0)),
        def("answer", [], Int, Expr::Int(42)),
    ]);

    assert_int(&program, "main", 42);
    assert_int(&program, "pick", 0);
}

#[test]
fn arguments_use_caller_slots_and_let_shadowing_is_lexical() {
    let scoped = bind("x", var("x"), var("x"));
    let body = call(var("second"), [Expr::Int(0), scoped]);
    let program = checked([
        def("second", [("a", Int), ("b", Int)], Int, var("b")),
        def("main", [], Int, bind("x", Expr::Int(42), body)),
    ]);

    assert_int(&program, "main", 42);
}

#[test]
fn rejects_wrong_function_signatures_and_checks_unused_bodies() {
    let unary = fun_ty([Int], Int);
    let errors = check(&source([
        def(
            "apply",
            [("f", unary.clone()), ("x", Int)],
            Int,
            call(var("f"), [var("x")]),
        ),
        def("wrong", [("x", Bool)], Int, Expr::Int(0)),
        def(
            "main",
            [],
            Int,
            call(var("apply"), [var("wrong"), Expr::Int(41)]),
        ),
        def(
            "unused",
            [],
            Int,
            branch(Expr::Bool(true), Expr::Int(1), Expr::Bool(false)),
        ),
    ]))
    .unwrap_err();

    assert_eq!(
        errors,
        [
            Diagnostic {
                function: "main".into(),
                error: TypeError::Mismatch {
                    expected: unary,
                    actual: fun_ty([Bool], Int)
                },
            },
            Diagnostic {
                function: "unused".into(),
                error: TypeError::Mismatch {
                    expected: Int,
                    actual: Bool
                },
            },
        ]
    );
}

#[test]
fn only_evaluates_selected_branch_and_caps_recursion() {
    let recurse = || call(var("recurse"), []);
    let program = checked([
        def("recurse", [], Int, recurse()),
        def(
            "main",
            [],
            Int,
            branch(Expr::Bool(true), Expr::Int(42), recurse()),
        ),
    ]);

    assert_int(&program, "main", 42);
    assert_eq!(
        program.run("recurse").unwrap_err(),
        ExecutionError::CallDepthExceeded
    );
}
