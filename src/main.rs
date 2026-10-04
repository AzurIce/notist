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
    /// Use this project configuration instead of searching parent directories
    #[arg(long, global = true)]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Parse and materialize a document, reporting every diagnostic
    Check { file: PathBuf },
    /// Print the lossless CST
    Cst { file: PathBuf },
    /// Print the desugared core tree
    Core { file: PathBuf },
    /// Print the JSON analysis (tree + ast + core + diagnostics)
    Json { file: PathBuf },
    /// Select elements by id, tag, ctor, function or level, JSON out
    Query { file: PathBuf, selector: String },
    /// Build HTML and the used component resources
    Html {
        file: PathBuf,
        #[arg(long, default_value = "target/notist-html")]
        out_dir: PathBuf,
    },
    /// Run the language server over stdio
    #[cfg(not(target_arch = "wasm32"))]
    Lsp,
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> ExitCode {
    let cli = Cli::parse();
    let config = cli.config.as_deref();
    match cli.command {
        Command::Check { file } => check(&file, config),
        Command::Cst { file } => print_with(&file, |src| {
            format!(
                "{:#?}",
                if is_module(&file) {
                    notist_syntax::parse_module(src)
                } else {
                    notist_syntax::parse_document(src)
                }
                .syntax()
            )
        }),
        Command::Core { file } if is_module(&file) => print_with(&file, |src| {
            format!("{:#?}\n", notist::analyze_module("package", src))
        }),
        Command::Core { file } => {
            let project = match load_project(&file, config) {
                Ok(project) => project,
                Err(code) => return code,
            };
            print_with(&file, |src| {
                let Ok(document) = project.analyzer().analyze(&file, src) else {
                    return format!("unsupported file extension: {}", file.display());
                };
                let mut out = String::new();
                for d in document.diagnostics() {
                    out.push_str(&format!(
                        "error[{}] @{}..{}: {}\n",
                        d.phase,
                        u32::from(d.span.start()),
                        u32::from(d.span.end()),
                        d.message
                    ));
                }
                out.push_str(&notist::dump::dump(document.root()));
                out
            })
        }
        Command::Json { file } if is_module(&file) => print_with(&file, |src| {
            notist::cst_json::analyze_module_json("package", src)
        }),
        Command::Json { file } => match load_project(&file, config) {
            Ok(project) => print_with(&file, |src| {
                notist::cst_json::analyze_document_json(&file, src, project.registry())
            }),
            Err(code) => code,
        },
        Command::Query { file, selector } => match read(&file) {
            Ok(src) => {
                let project = match load_project(&file, config) {
                    Ok(project) => project,
                    Err(code) => return code,
                };
                let Ok(document) = project.analyzer().analyze(&file, &src) else {
                    eprintln!("unsupported file extension: {}", file.display());
                    return ExitCode::FAILURE;
                };
                let matches = notist::query::select(document.root(), &selector);
                println!("{}", notist::query::render_json(&src, &matches));
                ExitCode::SUCCESS
            }
            Err(code) => code,
        },
        Command::Html { file, out_dir } => {
            let project = match load_project(&file, config) {
                Ok(project) => project,
                Err(code) => return code,
            };
            let src = match read(&file) {
                Ok(src) => src,
                Err(code) => return code,
            };
            let document = match project.analyzer().analyze(&file, &src) {
                Ok(document) => document,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
            };
            if !document.diagnostics().is_empty() {
                emit_source(&file, &src, document.diagnostics());
                return ExitCode::FAILURE;
            }
            match notist::html_host::build_page(document.root(), &project, &out_dir) {
                Ok(result) => {
                    println!("{}", result.page.display());
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error}");
                    ExitCode::FAILURE
                }
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        Command::Lsp => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(notist::lsp::serve_with_config(cli.config));
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
fn check(file: &Path, config: Option<&Path>) -> ExitCode {
    if file.is_dir() {
        return check_dir(file, config);
    }
    let src = match read(file) {
        Ok(src) => src,
        Err(code) => return code,
    };
    if is_module(file) {
        let diagnostics = notist::analyze_module("package", &src)
            .err()
            .unwrap_or_default();
        emit_source(file, &src, &diagnostics);
        return if diagnostics.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    let project = match load_project(file, config) {
        Ok(project) => project,
        Err(code) => return code,
    };
    let Ok(document) = project.analyzer().analyze(file, &src) else {
        eprintln!("unsupported file extension: {}", file.display());
        return ExitCode::FAILURE;
    };
    let (item, mut diagnostics) = document.into_parts();
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
fn check_dir(dir: &Path, config: Option<&Path>) -> ExitCode {
    let library = match notist::vault::Vault::load_with(dir, |path, src| {
        let project = notist::project::Project::for_document(path, config).map_err(|errors| {
            std::io::Error::other(
                errors
                    .iter()
                    .map(|e| format!("{}: {}", e.path.display(), e.diagnostic.message))
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        })?;
        Ok(project
            .analyzer()
            .analyze(path, src)
            .expect("supported document")
            .into_parts())
    }) {
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

#[cfg(not(target_arch = "wasm32"))]
fn is_module(file: &Path) -> bool {
    file.extension().is_some_and(|ext| ext == "notc")
}
#[cfg(not(target_arch = "wasm32"))]
fn load_project(file: &Path, config: Option<&Path>) -> Result<notist::project::Project, ExitCode> {
    notist::project::Project::for_document(file, config).map_err(|errors| {
        for error in errors {
            emit_source(&error.path, &error.source, &[error.diagnostic]);
        }
        ExitCode::FAILURE
    })
}
#[cfg(not(target_arch = "wasm32"))]
fn emit_source(path: &Path, source: &str, diagnostics: &[notist::Diagnostic]) {
    let files = SimpleFile::new(path.display().to_string(), source);
    let writer = StandardStream::stderr(ColorChoice::Auto);
    for error in diagnostics {
        let diagnostic = Diagnostic::error()
            .with_code(error.phase.to_string())
            .with_message(&error.message)
            .with_labels(vec![Label::primary(
                (),
                usize::from(error.span.start())..usize::from(error.span.end()),
            )]);
        let _ = term::emit(&mut writer.lock(), &Config::default(), &files, &diagnostic);
    }
}
