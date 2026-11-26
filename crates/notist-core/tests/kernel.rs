use Type::{Bool, Int};
use notist_core::{
    BinaryOp, CheckedProgram, Decl, Diagnostic, ExecutionError, Expr, ExprKind, Function, Method,
    Module, Param, Program, Span, Type, TypeError, Value, check,
};

fn def<const N: usize>(
    name: &str,
    parameters: [(&str, Type); N],
    returns: Type,
    body: Expr,
) -> Decl {
    Decl::Function(Function {
        name: name.into(),
        name_span: Span::DUMMY,
        type_params: Vec::new(),
        parameters: parameters
            .into_iter()
            .map(|(name, ty)| Param {
                name: name.into(),
                ty,
                default: None,
                span: Span::DUMMY,
            })
            .collect(),
        returns,
        where_clause: None,
        body: Some(body),
        span: Span::DUMMY,
    })
}

fn method<const N: usize>(
    receiver_type: Type,
    name: &str,
    receiver_name: &str,
    parameters: [(&str, Type); N],
    returns: Type,
    body: Expr,
) -> Decl {
    Decl::Method(Method {
        receiver_type,
        name: name.into(),
        name_span: Span::DUMMY,
        receiver_name: receiver_name.into(),
        receiver_span: Span::DUMMY,
        parameters: parameters
            .into_iter()
            .map(|(name, ty)| Param {
                name: name.into(),
                ty,
                default: None,
                span: Span::DUMMY,
            })
            .collect(),
        returns,
        body,
        span: Span::DUMMY,
    })
}

fn fun_ty<const N: usize>(parameters: [Type; N], returns: Type) -> Type {
    Type::Function(Vec::from(parameters), Box::new(returns))
}

fn int(value: i64) -> Expr {
    Expr::new(ExprKind::Int(value), Span::DUMMY)
}

fn bool_(value: bool) -> Expr {
    Expr::new(ExprKind::Bool(value), Span::DUMMY)
}

fn var(name: &str) -> Expr {
    Expr::new(ExprKind::Name(name.into()), Span::DUMMY)
}

fn call<const N: usize>(target: Expr, args: [Expr; N]) -> Expr {
    Expr::new(ExprKind::Call(Box::new(target), Vec::from(args)), Span::DUMMY)
}

fn call_method<const N: usize>(receiver: Expr, name: &str, args: [Expr; N]) -> Expr {
    Expr::new(
        ExprKind::MethodCall(Box::new(receiver), name.into(), Vec::from(args)),
        Span::DUMMY,
    )
}

fn add(left: Expr, right: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary(BinaryOp::Add, Box::new(left), Box::new(right)),
        Span::DUMMY,
    )
}

fn bind(name: &str, value: Expr, body: Expr) -> Expr {
    Expr::new(
        ExprKind::Let(name.into(), Box::new(value), Box::new(body)),
        Span::DUMMY,
    )
}

fn branch(condition: Expr, yes: Expr, no: Expr) -> Expr {
    Expr::new(
        ExprKind::If(Box::new(condition), Box::new(yes), Box::new(no)),
        Span::DUMMY,
    )
}

fn source<const N: usize>(declarations: [Decl; N]) -> Program {
    Program {
        root: Module {
            declarations: Vec::from(declarations),
        },
    }
}

fn checked<const N: usize>(declarations: [Decl; N]) -> CheckedProgram {
    check(&source(declarations)).expect("fixture should pass static checking")
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
        def("main", [], Int, call(returned, [int(41)])),
        def("identity", [("f", unary.clone())], unary, var("f")),
        def("inc", [("x", Int)], Int, add(var("x"), int(1))),
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
        def("pick", [], Int, int(0)),
        def("answer", [], Int, int(42)),
    ]);

    assert_int(&program, "main", 42);
    assert_int(&program, "pick", 0);
}

#[test]
fn arguments_use_caller_slots_and_let_shadowing_is_lexical() {
    let scoped = bind("x", var("x"), var("x"));
    let body = call(var("second"), [int(0), scoped]);
    let program = checked([
        def("second", [("a", Int), ("b", Int)], Int, var("b")),
        def("main", [], Int, bind("x", int(42), body)),
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
        def("wrong", [("x", Bool)], Int, int(0)),
        def("main", [], Int, call(var("apply"), [var("wrong"), int(41)])),
        def("unused", [], Int, branch(bool_(true), int(1), bool_(false))),
    ]))
    .unwrap_err();

    assert_eq!(
        errors,
        [
            Diagnostic {
                declaration: "main".into(),
                span: Span::DUMMY,
                error: TypeError::Mismatch {
                    expected: unary,
                    actual: fun_ty([Bool], Int)
                },
            },
            Diagnostic {
                declaration: "unused".into(),
                span: Span::DUMMY,
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
        def("main", [], Int, branch(bool_(true), int(42), recurse())),
    ]);

    assert_int(&program, "main", 42);
    assert_eq!(
        program.run("recurse").unwrap_err(),
        ExecutionError::CallDepthExceeded
    );
}

#[test]
fn resolves_and_chains_type_associated_methods_without_polluting_functions() {
    let program = checked([
        // Free function `inc` coexists cleanly with `Int.inc` and `Bool.inc`.
        def("inc", [("x", Int)], Int, add(var("x"), int(100))),
        method(Int, "inc", "self", [], Int, add(var("self"), int(1))),
        method(
            Int,
            "add_scaled",
            "self",
            [("delta", Int)],
            Int,
            add(call_method(var("self"), "inc", []), var("delta")),
        ),
        method(
            Bool,
            "inc",
            "self",
            [],
            Int,
            branch(var("self"), int(40), int(0)),
        ),
        def(
            "main",
            [],
            Int,
            // Local binding named `inc` cannot hijack `value.inc()`.
            bind(
                "inc",
                int(999),
                call_method(
                    call_method(bool_(true), "inc", []),
                    "add_scaled",
                    [int(1)],
                ),
            ),
        ),
    ]);

    assert_int(&program, "main", 42);
    // Methods are not exposed as top-level runnable entry points.
    assert_eq!(
        program.run("add_scaled").unwrap_err(),
        ExecutionError::UnknownEntry("add_scaled".into())
    );
}

#[test]
fn methods_can_return_first_class_functions_and_be_called_with_higher_order_args() {
    let unary = fun_ty([Int], Int);
    let program = checked([
        def("inc", [("x", Int)], Int, add(var("x"), int(1))),
        method(
            Int,
            "apply",
            "self",
            [("f", unary.clone())],
            Int,
            call(var("f"), [var("self")]),
        ),
        method(Bool, "pick_fn", "self", [], unary, var("inc")),
        def(
            "main",
            [],
            Int,
            call_method(int(41), "apply", [call_method(bool_(true), "pick_fn", [])]),
        ),
    ]);

    assert_int(&program, "main", 42);
}

#[test]
fn rejects_unknown_duplicate_and_ill_typed_methods() {
    let errors = check(&source([
        def("free_only", [("x", Int)], Int, var("x")),
        method(
            Int,
            "add_to",
            "self",
            [("n", Int)],
            Int,
            add(var("self"), var("n")),
        ),
        method(Int, "bad_unused", "self", [], Int, bool_(false)),
        method(Int, "dup_param", "self", [("self", Int)], Int, var("self")),
        def(
            "cannot_use_free_as_method",
            [],
            Int,
            call_method(int(1), "free_only", []),
        ),
        def(
            "wrong_receiver_type",
            [],
            Int,
            call_method(bool_(true), "add_to", [int(1)]),
        ),
        def("wrong_arity", [], Int, call_method(int(1), "add_to", [])),
        def(
            "wrong_arg_type",
            [],
            Int,
            call_method(int(1), "add_to", [bool_(true)]),
        ),
    ]))
    .unwrap_err();

    assert_eq!(
        errors,
        [
            Diagnostic {
                declaration: "Int.bad_unused".into(),
                span: Span::DUMMY,
                error: TypeError::Mismatch {
                    expected: Int,
                    actual: Bool,
                },
            },
            Diagnostic {
                declaration: "Int.dup_param".into(),
                span: Span::DUMMY,
                error: TypeError::DuplicateParameter("self".into()),
            },
            Diagnostic {
                declaration: "cannot_use_free_as_method".into(),
                span: Span::DUMMY,
                error: TypeError::UnknownMethod {
                    receiver: Int,
                    method: "free_only".into(),
                },
            },
            Diagnostic {
                declaration: "wrong_receiver_type".into(),
                span: Span::DUMMY,
                error: TypeError::UnknownMethod {
                    receiver: Bool,
                    method: "add_to".into(),
                },
            },
            Diagnostic {
                declaration: "wrong_arity".into(),
                span: Span::DUMMY,
                error: TypeError::Arity {
                    expected: 1,
                    actual: 0,
                },
            },
            Diagnostic {
                declaration: "wrong_arg_type".into(),
                span: Span::DUMMY,
                error: TypeError::Mismatch {
                    expected: Int,
                    actual: Bool,
                },
            },
        ]
    );
}

#[test]
fn rejects_duplicate_method_on_same_receiver_type() {
    let errors = check(&source([
        method(Int, "inc", "self", [], Int, var("self")),
        method(Int, "inc", "n", [], Int, var("n")),
    ]))
    .unwrap_err();

    assert_eq!(
        errors,
        [Diagnostic {
            declaration: "Int.inc".into(),
            span: Span::DUMMY,
            error: TypeError::DuplicateMethod,
        }]
    );
}
