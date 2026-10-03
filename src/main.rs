#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use codespan_reporting::diagnostic::{Diagnostic, Label};
use codespan_reporting::files::{SimpleFile, SimpleFiles};
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};
use codespan_reporting::term::{self, Config};

#[derive(Parser)]
#[command(version, about = "the Notist document toolchain", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse and evaluate a document, reporting every diagnostic
    Check { file: PathBuf },
    /// Print the lossless CST
    Cst { file: PathBuf },
    /// Print the desugared core tree
    Core { file: PathBuf },
    /// Print the JSON analysis (tree + ast + core + diagnostics)
    Json { file: PathBuf },
    /// Select elements by an `id:` / `tag:` / `ctor:` selector, JSON out
    Query { file: PathBuf, selector: String },
    /// Run the language server over stdio
    #[cfg(not(target_arch = "wasm32"))]
    Lsp,
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check { file } => check(&file),
        Command::Cst { file } => print_with(&file, |src| {
            format!("{:#?}", notist_syntax::parser::parse(src).syntax())
        }),
        Command::Core { file } => print_with(&file, |src| {
            let Some((item, diags)) = notist::frontend::Frontends::default().analyze(&file, src)
            else {
                return format!("unsupported file extension: {}", file.display());
            };
            let mut out = String::new();
            for d in &diags {
                out.push_str(&format!(
                    "error[{}] @{}..{}: {}\n",
                    d.phase,
                    u32::from(d.span.start()),
                    u32::from(d.span.end()),
                    d.message
                ));
            }
            out.push_str(&notist::dump::dump(&item));
            out
        }),
        Command::Json { file } => print_with(&file, notist::cst_json::analyze_json),
        Command::Query { file, selector } => match read(&file) {
            Ok(src) => {
                let Some((item, _)) = notist::frontend::Frontends::default().analyze(&file, &src)
                else {
                    eprintln!("unsupported file extension: {}", file.display());
                    return ExitCode::FAILURE;
                };
                let matches = notist::query::select(&item, &selector);
                println!("{}", notist::query::render_json(&src, &matches));
                ExitCode::SUCCESS
            }
            Err(code) => code,
        },
        #[cfg(not(target_arch = "wasm32"))]
        Command::Lsp => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(notist::lsp::serve());
            ExitCode::SUCCESS
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read(file: &Path) -> Result<String, ExitCode> {
    std::fs::read_to_string(file).map_err(|err| {
        eprintln!("{}: {err}", file.display());
        ExitCode::FAILURE
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn print_with(file: &Path, f: impl Fn(&str) -> String) -> ExitCode {
    match read(file) {
        Ok(src) => {
            print!("{}", f(&src));
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn check(file: &Path) -> ExitCode {
    if file.is_dir() {
        return check_dir(file);
    }
    let src = match read(file) {
        Ok(src) => src,
        Err(code) => return code,
    };
    let Some((item, mut diagnostics)) = notist::frontend::Frontends::default().analyze(file, &src)
    else {
        eprintln!("unsupported file extension: {}", file.display());
        return ExitCode::FAILURE;
    };
    // id/tag 约定是 .not 的注解语义，只在该前端下检查
    if file.extension().is_some_and(|ext| ext == "not") {
        let index = notist::index::Index::build(&item, &mut diagnostics);
        notist::vault::check_doc_links(&item, &index, &mut diagnostics);
    }
    let files = SimpleFile::new(file.display().to_string(), &src);
    let writer = StandardStream::stderr(ColorChoice::Auto);
    let config = Config::default();
    for d in &diagnostics {
        let range = usize::from(d.span.start())..usize::from(d.span.end());
        let diagnostic = Diagnostic::error()
            .with_code(d.phase.to_string())
            .with_message(&d.message)
            .with_labels(vec![Label::primary((), range)]);
        if term::emit(&mut writer.lock(), &config, &files, &diagnostic).is_err() {
            return ExitCode::FAILURE;
        }
    }
    if diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "check: {} issue{} in {}",
            diagnostics.len(),
            if diagnostics.len() == 1 { "" } else { "s" },
            file.display()
        );
        ExitCode::FAILURE
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn check_dir(dir: &Path) -> ExitCode {
    let library = match notist::vault::Vault::load(dir) {
        Ok(library) => library,
        Err(err) => {
            eprintln!("{}: {err}", dir.display());
            return ExitCode::FAILURE;
        }
    };
    let diagnostics = library.check();
    let mut files = SimpleFiles::new();
    let mut ids = std::collections::HashMap::new();
    let writer = StandardStream::stderr(ColorChoice::Auto);
    let config = Config::default();
    for (path, d) in &diagnostics {
        let id = *ids.entry(path.clone()).or_insert_with(|| {
            let full = dir.join(path);
            let src = std::fs::read_to_string(&full).unwrap_or_default();
            files.add(full.display().to_string(), src)
        });
        let range = usize::from(d.span.start())..usize::from(d.span.end());
        let diagnostic = Diagnostic::error()
            .with_code(d.phase.to_string())
            .with_message(&d.message)
            .with_labels(vec![Label::primary(id, range)]);
        let _ = term::emit(&mut writer.lock(), &config, &files, &diagnostic);
    }
    if diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "check: {} issue{} under {}",
            diagnostics.len(),
            if diagnostics.len() == 1 { "" } else { "s" },
            dir.display()
        );
        ExitCode::FAILURE
    }
}