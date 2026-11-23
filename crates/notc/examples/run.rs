//! Run a `.notc` file (single-file or multi-file via relative `import`) and
//! print diagnostics with line/column spans.
//!
//! Usage:
//!   cargo run -p notc --example run -- <path.notc> [entry]

use notc::{
    Decl, ProjectDiagnosticKind, Span, TypeError, Value, check_project, parse,
    resolve_import_path,
};
use std::collections::{HashMap, VecDeque};
use std::{env, fs, path::Path, process::ExitCode};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(entry_arg) = args.next() else {
        eprintln!("usage: run <file.notc> [entry]");
        return ExitCode::from(2);
    };
    let entry_fn = args.next().unwrap_or_else(|| "main".into());

    let entry_fs = Path::new(&entry_arg);
    let root_dir = entry_fs.parent().unwrap_or_else(|| Path::new("."));
    let Some(entry_name) = entry_fs.file_name().and_then(|s| s.to_str()) else {
        eprintln!("error: invalid entry path `{entry_arg}`");
        return ExitCode::FAILURE;
    };

    // Host responsibility: discover and read transitively imported `.notc` files.
    // The `notc` library itself performs zero filesystem IO.
    let mut loaded: Vec<(String, String)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    seen.insert(entry_name.to_owned(), 0);
    queue.push_back(entry_name.to_owned());

    while let Some(rel) = queue.pop_front() {
        let fs_path = root_dir.join(&rel);
        let source = match fs::read_to_string(&fs_path) {
            Ok(text) => text,
            Err(err) => {
                if rel == entry_name {
                    eprintln!("error: failed to read `{}`: {err}", fs_path.display());
                    return ExitCode::FAILURE;
                }
                // Let `notc::check_project` report `MissingModule` with the exact import span.
                continue;
            }
        };
        let parsed = parse(&source);
        for decl in &parsed.program.root.declarations {
            if let Decl::Import(import) = decl
                && let Ok(target) = resolve_import_path(&rel, &import.path)
                && !seen.contains_key(&target)
            {
                let next_idx = seen.len();
                seen.insert(target.clone(), next_idx);
                queue.push_back(target);
            }
        }
        loaded.push((rel, source));
    }

    let source_by_module: HashMap<&str, &str> = loaded
        .iter()
        .map(|(p, s)| (p.as_str(), s.as_str()))
        .collect();
    let files: Vec<(&str, &str)> = loaded
        .iter()
        .map(|(p, s)| (p.as_str(), s.as_str()))
        .collect();

    let project = match check_project(&files) {
        Ok(project) => project,
        Err(diagnostics) => {
            for diag in &diagnostics {
                let display_path = root_dir.join(&diag.module);
                let display = display_path.to_string_lossy();
                let source = source_by_module
                    .get(diag.module.as_str())
                    .copied()
                    .unwrap_or("");
                match &diag.kind {
                    ProjectDiagnosticKind::Parse(parse_diag) => {
                        print_diagnostic(
                            &display,
                            source,
                            parse_diag.span,
                            "parse error",
                            &parse_diag.message,
                        );
                    }
                    ProjectDiagnosticKind::Type {
                        declaration,
                        span,
                        error,
                    } => {
                        let message = format!("in `{declaration}`: {}", format_type_error(error));
                        print_diagnostic(&display, source, *span, "type error", &message);
                    }
                }
            }
            return ExitCode::FAILURE;
        }
    };

    match project.run(entry_name, &entry_fn) {
        Ok(value) => {
            println!("{entry_fn}() => {}", format_value(value));
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("runtime error in `{entry_fn}`: {err:?}");
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

fn format_type_error(error: &TypeError) -> String {
    notc::format_type_error(error)
}

fn format_value(value: Value) -> String {
    match value {
        Value::Unit => "()".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => format!("{s:?}"),
        Value::Inline(inline) => inline.to_string(),
        Value::Block(block) => block.to_string(),
        Value::Function(_) => "<function>".into(),
        Value::Builtin(b, _) => format!("<builtin:{b:?}>"),
    }
}
