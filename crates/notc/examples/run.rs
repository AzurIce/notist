//! Run a standalone `.notc` file and print diagnostics with line/column spans.
//!
//! Usage:
//!   cargo run -p notc --example run -- <path.notc> [entry]

use notc::{Span, Type, TypeError, Value, check, parse};
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: run <file.notc> [entry]");
        return ExitCode::from(2);
    };
    let entry = args.next().unwrap_or_else(|| "main".into());

    let source = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: failed to read `{path}`: {err}");
            return ExitCode::FAILURE;
        }
    };

    let parsed = parse(&source);
    let mut has_errors = false;
    for diag in &parsed.diagnostics {
        has_errors = true;
        print_diagnostic(&path, &source, diag.span, "parse error", &diag.message);
    }

    let checked = match check(&parsed.program) {
        Ok(program) => Some(program),
        Err(diagnostics) => {
            has_errors = true;
            for diag in &diagnostics {
                let message = format!(
                    "in `{}`: {}",
                    diag.declaration,
                    format_type_error(&diag.error)
                );
                print_diagnostic(&path, &source, diag.span, "type error", &message);
            }
            None
        }
    };

    if has_errors {
        return ExitCode::FAILURE;
    }

    let program = checked.expect("error-free check produces a CheckedProgram");
    match program.run(&entry) {
        Ok(value) => {
            println!("{entry}() => {}", format_value(value));
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("runtime error in `{entry}`: {err:?}");
            ExitCode::FAILURE
        }
    }
}

fn print_diagnostic(path: &str, source: &str, span: Span, kind: &str, message: &str) {
    let start = (span.start as usize).min(source.len());
    let end = (span.end as usize).min(source.len()).max(start);
    let line_start = source[..start].rfind('\n').map_or(0, |i| i + 1);
    let line_end = source[start..]
        .find('\n')
        .map_or(source.len(), |i| start + i);
    let line_no = source[..line_start].bytes().filter(|&b| b == b'\n').count() + 1;
    let col_no = source[line_start..start].chars().count() + 1;

    let line_text = &source[line_start..line_end];
    let highlight_end = end.min(line_end);
    let caret_len = source[start..highlight_end].chars().count().max(1);
    let pad = " ".repeat(col_no.saturating_sub(1));
    let carets = "^".repeat(caret_len);

    eprintln!("{kind}: {message}");
    eprintln!("  --> {path}:{line_no}:{col_no}");
    eprintln!("   |");
    eprintln!("{line_no:>2} | {line_text}");
    eprintln!("   | {pad}{carets}");
}

fn format_type(ty: &Type) -> String {
    match ty {
        Type::Int => "Int".into(),
        Type::Bool => "Bool".into(),
        Type::Function(params, ret) => {
            let params = params
                .iter()
                .map(format_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("fn({params}) -> {}", format_type(ret))
        }
    }
}

fn format_type_error(error: &TypeError) -> String {
    match error {
        TypeError::DuplicateFunction => "duplicate function declaration".into(),
        TypeError::DuplicateMethod => "duplicate method declaration for receiver type".into(),
        TypeError::DuplicateParameter(name) => format!("duplicate parameter `{name}`"),
        TypeError::UnknownName(name) => format!("unknown name `{name}`"),
        TypeError::UnknownMethod { receiver, method } => {
            format!("no method `{method}` on type `{}`", format_type(receiver))
        }
        TypeError::NotCallable(ty) => format!("type `{}` is not callable", format_type(ty)),
        TypeError::Arity { expected, actual } => {
            format!("expected {expected} argument(s), found {actual}")
        }
        TypeError::Mismatch { expected, actual } => format!(
            "type mismatch: expected `{}`, found `{}`",
            format_type(expected),
            format_type(actual)
        ),
    }
}

fn format_value(value: Value) -> String {
    match value {
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Function(_) => "<function>".into(),
    }
}
