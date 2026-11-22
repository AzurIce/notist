use notc::{
    ProjectDiagnosticKind, Type, TypeError, Value, check_project,
};

#[test]
fn supports_all_import_forms_reexports_and_cross_module_methods() {
    let project = check_project(&[
        (
            "lib/core.notc",
            r#"
fn base() -> Int { 35 }
fn inc(x: Int) -> Int { x + 1 }
fn Int.plus_two(self) -> Int { self + 2 }
fn (fn(Int) -> Int).twice(self, x: Int) -> Int { self(self(x)) }
fn unused_dead_fn() -> Int { 999 }
"#,
        ),
        (
            "lib/reexport.notc",
            r#"
import "./core.notc" as core;
import "./core.notc": inc as add_one;
fn wrap(x: Int) -> Int { add_one(x) }
"#,
        ),
        (
            "lib/wild.notc",
            r#"
fn bonus() -> Int { 2 }
"#,
        ),
        (
            "main.notc",
            r#"
import "./lib/core.notc";
import "./lib/reexport.notc" as re;
import "./lib/wild.notc": *;

fn main() -> Int {
    // Default alias `core`, nested re-exported namespace `re.core`,
    // wildcard `bonus`, method `Int.plus_two`, and method on a function value `core.inc.twice`.
    let start = re.core.base();
    let stepped = core.inc.twice(start);
    let added = re.wrap(stepped).plus_two();
    added + bonus()
}
"#,
        ),
    ])
    .expect("multi-module project should check");

    // 35 + 2 (twice inc) + 1 (re.wrap) + 2 (plus_two) + 2 (bonus) = 42.
    assert!(matches!(project.run("main.notc", "main"), Ok(Value::Int(42))));

    // Verify reachability-driven lowering excludes `unused_dead_fn`.
    let lowered = project
        .lower_entry("main.notc", "main")
        .expect("main should lower");
    // Reachable: main, base, inc, twice, wrap, plus_two, bonus = 7 (not 8).
    assert_eq!(lowered.function_count(), 7);
}

#[test]
fn local_binding_shadows_imported_namespace_for_method_calls() {
    let project = check_project(&[
        (
            "math.notc",
            r#"
fn inc(x: Int) -> Int { x + 100 }
fn Int.inc(self) -> Int { self + 1 }
"#,
        ),
        (
            "main.notc",
            r#"
import "./math.notc";

fn main() -> Int {
    let math = 41;
    math.inc()
}
"#,
        ),
    ])
    .expect("project should check");

    assert!(matches!(project.run("main.notc", "main"), Ok(Value::Int(42))));
}

#[test]
fn checks_unused_functions_across_all_modules_before_lowering() {
    let errors = check_project(&[
        (
            "helper.notc",
            r#"
fn good() -> Int { 42 }
fn broken_unused() -> Int { false }
"#,
        ),
        (
            "main.notc",
            r#"
import "./helper.notc": good;
fn main() -> Int { good() }
"#,
        ),
    ])
    .unwrap_err();

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].module, "helper.notc");
    assert!(matches!(
        errors[0].kind,
        ProjectDiagnosticKind::Type {
            ref declaration,
            error: TypeError::Mismatch {
                expected: Type::Int,
                actual: Type::Bool,
            },
            ..
        } if declaration == "broken_unused"
    ));
}

#[test]
fn detects_static_import_cycles() {
    let errors = check_project(&[
        ("a.notc", r#"import "./b.notc"; fn a() -> Int { 1 }"#),
        ("b.notc", r#"import "./c.notc"; fn b() -> Int { 2 }"#),
        ("c.notc", r#"import "./a.notc"; fn c() -> Int { 3 }"#),
    ])
    .unwrap_err();

    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].module, "c.notc");
    assert!(matches!(
        errors[0].kind,
        ProjectDiagnosticKind::Type {
            error: TypeError::ImportCycle(ref cycle),
            ..
        } if cycle == &["a.notc", "b.notc", "c.notc", "a.notc"]
    ));
}

#[test]
fn rejects_missing_module_unknown_export_duplicate_binding_and_ambiguous_method() {
    let missing = check_project(&[(
        "main.notc",
        r#"import "./nope.notc"; fn main() -> Int { 0 }"#,
    )])
    .unwrap_err();
    assert!(matches!(
        missing[0].kind,
        ProjectDiagnosticKind::Type {
            error: TypeError::MissingModule(ref p),
            ..
        } if p == "nope.notc"
    ));

    let unknown = check_project(&[
        ("lib.notc", "fn a() -> Int { 1 }"),
        (
            "main.notc",
            r#"import "./lib.notc": nope; fn main() -> Int { 0 }"#,
        ),
    ])
    .unwrap_err();
    assert!(matches!(
        unknown[0].kind,
        ProjectDiagnosticKind::Type {
            error: TypeError::UnknownExport { ref module, ref name },
            ..
        } if module == "lib.notc" && name == "nope"
    ));

    let dup = check_project(&[
        ("a.notc", "fn f() -> Int { 1 }"),
        ("b.notc", "fn f() -> Int { 2 }"),
        (
            "main.notc",
            r#"
import "./a.notc": f;
import "./b.notc": *;
fn main() -> Int { f() }
"#,
        ),
    ])
    .unwrap_err();
    assert!(matches!(
        dup[0].kind,
        ProjectDiagnosticKind::Type {
            error: TypeError::DuplicateBinding(ref name),
            ..
        } if name == "f"
    ));

    let ambig = check_project(&[
        ("a.notc", "fn Int.inc(self) -> Int { self + 1 }"),
        ("b.notc", "fn Int.inc(self) -> Int { self + 2 }"),
        (
            "main.notc",
            r#"
import "./a.notc";
import "./b.notc";
fn main() -> Int { 1.inc() }
"#,
        ),
    ])
    .unwrap_err();
    assert!(matches!(
        ambig[0].kind,
        ProjectDiagnosticKind::Type {
            error: TypeError::AmbiguousMethod {
                receiver: Type::Int,
                ref method,
            },
            ..
        } if method == "inc"
    ));
}

#[test]
fn rejects_namespace_used_as_value_and_member_access_on_non_namespace() {
    let errors = check_project(&[
        ("math.notc", "fn inc(x: Int) -> Int { x + 1 }"),
        (
            "main.notc",
            r#"
import "./math.notc";
fn ns_value() -> Int { let x = math; 0 }
fn val_member() -> Int { 42.foo }
"#,
        ),
    ])
    .unwrap_err();

    assert_eq!(errors.len(), 2);
    assert!(matches!(
        errors[0].kind,
        ProjectDiagnosticKind::Type {
            error: TypeError::NamespaceAsValue(ref name),
            ..
        } if name == "math"
    ));
    assert!(matches!(
        errors[1].kind,
        ProjectDiagnosticKind::Type {
            error: TypeError::NotNamespace(Type::Int),
            ..
        }
    ));
}
