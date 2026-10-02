use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use codespan_reporting::diagnostic::{Diagnostic, Label};
use codespan_reporting::files::SimpleFile;
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
    /// Run the language server over stdio
    #[cfg(not(target_arch = "wasm32"))]
    Lsp,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check { file } => check(&file),
        Command::Cst { file } => print_with(&file, |src| {
            format!("{:#?}", notist_syntax::parser::parse(src).syntax())
        }),
        Command::Core { file } => print_with(&file, notist::dump_str),
        Command::Json { file } => print_with(&file, notist::cst_json::analyze_json),
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

fn read(file: &Path) -> Result<String, ExitCode> {
    std::fs::read_to_string(file).map_err(|err| {
        eprintln!("{}: {err}", file.display());
        ExitCode::FAILURE
    })
}

fn print_with(file: &Path, f: impl Fn(&str) -> String) -> ExitCode {
    match read(file) {
        Ok(src) => {
            print!("{}", f(&src));
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

fn check(file: &Path) -> ExitCode {
    let src = match read(file) {
        Ok(src) => src,
        Err(code) => return code,
    };
    let (_item, diagnostics) = notist::analyze(&src);
    let files = SimpleFile::new(file.display().to_string(), &src);
    let writer = StandardStream::stderr(ColorChoice::Auto);
    let config = Config::default();
    for d in &diagnostics {
        let range = usize::from(d.span.start())..usize::from(d.span.end());
        let diagnostic = Diagnostic::error()
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
