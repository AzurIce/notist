#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
mod html;
#[cfg(all(not(target_arch = "wasm32"), feature = "lsp"))]
mod lsp;
#[cfg(not(target_arch = "wasm32"))]
mod package;
#[cfg(not(target_arch = "wasm32"))]
mod progress;
#[cfg(not(target_arch = "wasm32"))]
mod publication;
mod query;
#[cfg(not(target_arch = "wasm32"))]
mod serve;
#[cfg(not(target_arch = "wasm32"))]
mod ssg;
mod timings;

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use codespan_reporting::diagnostic::{Diagnostic, Label, Severity};
use codespan_reporting::files::SimpleFile;
use codespan_reporting::term::termcolor::{ColorChoice, StandardStream};
use codespan_reporting::term::{self, Config};

#[derive(Parser)]
#[command(name = "notist", version, about = "the Notist document toolchain", long_about = None)]
struct Cli {
    /// Record stage timings (html by default; use --timings=html,json)
    #[arg(long, global = true, num_args = 0..=1, require_equals = true,
        default_missing_value = "html", value_delimiter = ',', value_enum)]
    timings: Vec<timings::Format>,
    /// Show detailed, tree-shaped build progress on stderr
    #[arg(short, long, global = true)]
    verbose: bool,
    /// Maximum build workers shared by all stages (default: available CPUs)
    #[arg(short = 'j', long, global = true, value_name = "N")]
    jobs: Option<std::num::NonZeroUsize>,
    #[command(subcommand)]
    command: Command,
    /// Use this Notist.toml instead of automatic configuration discovery
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Inspect the representation tree after Code evaluation and extensions
    #[arg(long, global = true)]
    eval: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Build the current package's site
    Build {
        /// Build every workspace package that declares a site
        #[arg(long)]
        workspace: bool,
        /// Select workspace packages by name (repeatable)
        #[arg(short = 'p', long = "package", conflicts_with = "workspace")]
        packages: Vec<String>,
        /// Override site output (relative to cwd)
        #[arg(long)]
        out_dir: Option<PathBuf>,
    },
    /// Preview the current package's site
    Serve {
        /// Select a workspace package by name
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
        #[command(flatten)]
        listen: ListenOptions,
        #[arg(long)]
        no_clear_screen: bool,
    },
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
    #[cfg(all(not(target_arch = "wasm32"), feature = "lsp"))]
    Lsp,
}

#[derive(Args)]
struct ListenOptions {
    /// Host or IP address to listen on
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    /// Listen port; tries 8000, 8001, ... by default (0 selects any free port)
    #[arg(long)]
    port: Option<u16>,
    /// Legacy host:port syntax; cannot be combined with --host or --port
    #[arg(long, conflicts_with_all = ["host", "port"])]
    address: Option<String>,
}

#[cfg(test)]
mod jobs_tests {
    use super::*;

    #[test]
    fn jobs_is_global_and_uses_one_positive_value() {
        for args in [
            vec!["notist", "-j", "3", "build"],
            vec!["notist", "build", "--jobs", "3"],
            vec!["notist", "serve", "-j3"],
            vec!["notist", "-j3", "build", "--workspace"],
        ] {
            assert_eq!(Cli::try_parse_from(args).unwrap().jobs.unwrap().get(), 3);
        }
        assert!(
            Cli::try_parse_from(["notist", "build"])
                .unwrap()
                .jobs
                .is_none()
        );
        for value in ["0", "-1", "invalid"] {
            assert!(Cli::try_parse_from(["notist", "build", "--jobs", value]).is_err());
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> ExitCode {
    let cli = Cli::parse();
    progress::observe(cli.verbose, || {
        if cli.timings.is_empty() || matches!(&cli.command, Command::Serve { .. }) {
            return run(cli);
        }
        #[cfg(feature = "lsp")]
        if matches!(&cli.command, Command::Lsp) {
            eprintln!("--timings is not supported for the long-running language server");
            return ExitCode::FAILURE;
        }
        let formats = cli.timings.clone();
        let (result, mut report) = notist::progress::timings::capture(|| run(cli));
        report.attributes.insert(
            "status".into(),
            if result == ExitCode::SUCCESS {
                "success"
            } else {
                "failed"
            }
            .into(),
        );
        if let Err(error) = timings::write(&report, &formats) {
            eprintln!("Failed to write timings: {error}");
            return ExitCode::FAILURE;
        }
        result
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn run(cli: Cli) -> ExitCode {
    let config = cli.config.as_deref();
    let evaluate = cli.eval;
    match cli.command {
        Command::Build {
            out_dir,
            workspace,
            packages,
        } => package::build(config, out_dir.as_deref(), workspace, &packages, cli.jobs),
        Command::Serve {
            package,
            listen,
            no_clear_screen,
        } => match package::Package::load(config, None, package.as_deref()) {
            Ok(package) => serve::serve_package(
                package,
                &listen,
                no_clear_screen,
                cli.verbose,
                cli.jobs,
                !cli.timings.is_empty(),
            ),
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        Command::Check { file } => check(&file, config),
        Command::Cst { file } => print_with(&file, |src| {
            format!(
                "{:#?}",
                if is_module(&file) {
                    notist::syntax::code::parse_module(
                        src,
                        &notist::syntax::markup::MarkupContentParser,
                    )
                } else {
                    notist::syntax::markup::parse_document(src)
                }
                .syntax()
            )
        }),
        Command::Core { file } if is_module(&file) => {
            let vault = load_vault(config);
            print_with(&file, |src| {
                format!("{:#?}\n", vault.analyze_code(&file, src))
            })
        }
        Command::Core { file } => {
            let mut vault = load_vault(config);
            print_result(&file, |src| {
                let document = vault.analyze(&file, src).map_err(emit_vault_error)?;
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
                if evaluate {
                    match vault.evaluate(&file, &document) {
                        Ok(e) => {
                            for d in e.diagnostics {
                                out.push_str(&format!("error[{}]: {}\n", d.phase, d.message));
                            }
                            out.push_str(
                                &notist::dump::dump(&e.root, &e.sources)
                                    .unwrap_or_else(|error| error.to_string()),
                            );
                        }
                        Err(e) => out.push_str(&e.to_string()),
                    }
                } else {
                    out.push_str(
                        &notist::dump::dump(document.root(), document.sources())
                            .unwrap_or_else(|error| error.to_string()),
                    );
                }
                Ok(out)
            })
        }
        Command::Json { file } if is_module(&file) => {
            let vault = load_vault(config);
            print_with(&file, |src| match vault.analyze_code(&file, src) {
                Ok(analysis) => notist::cst_json::module_json(src, &analysis),
                Err(error) => serde_json::json!({"error":error.to_string()}).to_string(),
            })
        }
        Command::Json { file } => {
            let mut vault = load_vault(config);
            print_result(&file, |src| match vault.inspect(&file, src) {
                Ok((analysis, inspection)) => {
                    let text = notist::cst_json::inspection_json(&analysis, Some(&inspection))
                        .map_err(|error| {
                            eprintln!("{error}");
                            ExitCode::FAILURE
                        })?;
                    if evaluate {
                        let mut data: serde_json::Value = serde_json::from_str(&text).unwrap();
                        match vault.evaluate(&file, &analysis) {
                            Ok(e) => {
                                data["core"] =
                                    notist::json::item(&e.root, &e.sources).map_err(|error| {
                                        eprintln!("{error}");
                                        ExitCode::FAILURE
                                    })?;
                                data["evaluation_diagnostics"] = serde_json::json!(
                                    e.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
                                );
                            }
                            Err(e) => data["error"] = e.to_string().into(),
                        };
                        Ok(data.to_string())
                    } else {
                        Ok(text)
                    }
                }
                Err(error) => Err(emit_vault_error(error)),
            })
        }
        Command::Query { file, selector } => match read(&file) {
            Ok(src) => {
                let mut vault = load_vault(config);
                let document = match vault.analyze(&file, &src) {
                    Ok(document) => document,
                    Err(error) => return emit_vault_error(error),
                };
                let evaluation = if evaluate {
                    match vault.evaluate(&file, &document) {
                        Ok(e) => Some(e),
                        Err(e) => {
                            eprintln!("{e}");
                            return ExitCode::FAILURE;
                        }
                    }
                } else {
                    None
                };
                let root = evaluation.as_ref().map_or(document.root(), |e| &e.root);
                let sources = evaluation
                    .as_ref()
                    .map_or(document.sources(), |e| &e.sources);
                let matches = match notist::query::select(root, &selector, sources) {
                    Ok(matches) => matches,
                    Err(error) => {
                        eprintln!("{error}");
                        return ExitCode::FAILURE;
                    }
                };
                match query::render_json(sources, &matches) {
                    Ok(output) => println!("{output}"),
                    Err(error) => {
                        eprintln!("{error}");
                        return ExitCode::FAILURE;
                    }
                }
                ExitCode::SUCCESS
            }
            Err(code) => code,
        },
        Command::Html { file, out_dir } => render_html_notc(&file, &out_dir),
        #[cfg(all(not(target_arch = "wasm32"), feature = "lsp"))]
        Command::Lsp => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            runtime.block_on(lsp::serve(cli.config));
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
fn print_with(file: &Path, mut f: impl FnMut(&str) -> String) -> ExitCode {
    match read(file) {
        Ok(src) => {
            print!("{}", f(&src));
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn print_result(file: &Path, mut f: impl FnMut(&str) -> Result<String, ExitCode>) -> ExitCode {
    match read(file).and_then(|src| f(&src)) {
        Ok(output) => {
            print!("{output}");
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn load_notc_graph(
    entry: &Path,
) -> Result<(PathBuf, String, Vec<(String, PathBuf, String)>), ExitCode> {
    use std::collections::{HashSet, VecDeque};
    let abs = std::fs::canonicalize(entry).unwrap_or_else(|_| entry.to_path_buf());
    let root = if abs.is_dir() {
        abs.clone()
    } else {
        abs.parent().unwrap_or(Path::new(".")).to_path_buf()
    };

    let mut files: Vec<(String, PathBuf, String)> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();

    if abs.is_dir() {
        fn walk_dir(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            paths.sort();
            for p in paths {
                if p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                {
                    continue;
                }
                if p.is_dir() {
                    walk_dir(root, &p, out);
                } else if p
                    .extension()
                    .is_some_and(|e| e == "not" || e == "notc" || e == "md")
                {
                    out.push(p);
                }
            }
        }
        let mut discovered = Vec::new();
        walk_dir(&root, &abs, &mut discovered);
        for fs_path in discovered {
            let rel = fs_path
                .strip_prefix(&root)
                .unwrap_or(&fs_path)
                .to_string_lossy()
                .replace('\\', "/");
            if let Ok(src) = std::fs::read_to_string(&fs_path)
                && visited.insert(rel.clone())
            {
                queue.push_back(rel.clone());
                files.push((rel, fs_path, src));
            }
        }
    } else {
        let src = read(&abs)?;
        let rel = abs
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        visited.insert(rel.clone());
        queue.push_back(rel.clone());
        files.push((rel, abs.clone(), src));
    }

    let entry_rel = files.first().map(|(r, _, _)| r.clone()).unwrap_or_default();

    let frontends: [&dyn notc::DocumentFrontend; 2] =
        [&notc::NotistFrontend, &notist_markdown::MarkdownFrontend];

    // Transitively load imported `.notc` modules.
    while let Some(rel) = queue.pop_front() {
        let Some(src) = files.iter().find(|(r, _, _)| r == &rel).map(|(_, _, s)| s.clone()) else {
            continue;
        };
        let parsed = notc::parse_file_with_frontends(&rel, &src, &frontends);
        for decl in &parsed.program.root.declarations {
            if let notc::Decl::Import(import) = decl
                && let Ok(target_rel) = notc::resolve_import_path(&rel, &import.path)
                && visited.insert(target_rel.clone())
            {
                let target_fs = root.join(&target_rel);
                if let Ok(target_src) = std::fs::read_to_string(&target_fs) {
                    queue.push_back(target_rel.clone());
                    files.push((target_rel, target_fs, target_src));
                }
            }
        }
    }

    Ok((root, entry_rel, files))
}

#[cfg(not(target_arch = "wasm32"))]
fn emit_notc_diagnostics(
    files: &[(String, PathBuf, String)],
    diagnostics: &[notc::ide::IdeDiagnostic],
) {
    let writer = StandardStream::stderr(stderr_color_choice());
    let mut lock = writer.lock();
    let cfg = Config::default();
    for d in diagnostics {
        let (display_path, src) = files
            .iter()
            .find(|(rel, _, _)| rel == &d.module)
            .map(|(_, fs, s)| (fs.display().to_string(), s.as_str()))
            .unwrap_or_else(|| (d.module.clone(), ""));
        let simple = SimpleFile::new(display_path, src);
        let phase = match d.phase {
            notc::ide::DiagnosticPhase::Parse => "syntax",
            notc::ide::DiagnosticPhase::Type => "type",
        };
        let start = (d.span.start as usize).min(src.len());
        let end = (d.span.end as usize).min(src.len()).max(start);
        let diag = Diagnostic::error()
            .with_code(phase)
            .with_message(&d.message)
            .with_labels(vec![Label::primary((), start..end)]);
        let _ = term::emit(&mut lock, &cfg, &simple, &diag);
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn check(file: &Path, _config: Option<&Path>) -> ExitCode {
    let Ok((_, _, files)) = load_notc_graph(file) else {
        return ExitCode::FAILURE;
    };
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, _, src)| (rel.as_str(), src.as_str()))
        .collect();
    let frontends: [&dyn notc::DocumentFrontend; 2] =
        [&notc::NotistFrontend, &notist_markdown::MarkdownFrontend];
    let analysis = notc::ide::WorkspaceAnalysis::analyze_with_frontends(&pairs, &frontends);
    let diags = analysis.diagnostics();
    if diags.is_empty() {
        ExitCode::SUCCESS
    } else {
        emit_notc_diagnostics(&files, diags);
        eprintln!(
            "check: {} issue{} in {}",
            diags.len(),
            if diags.len() == 1 { "" } else { "s" },
            file.display()
        );
        ExitCode::FAILURE
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn render_html_notc(file: &Path, out_dir: &Path) -> ExitCode {
    let Ok((_, entry_rel, files)) = load_notc_graph(file) else {
        return ExitCode::FAILURE;
    };
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, _, src)| (rel.as_str(), src.as_str()))
        .collect();
    let frontends: [&dyn notc::DocumentFrontend; 2] =
        [&notc::NotistFrontend, &notist_markdown::MarkdownFrontend];
    let parsed_files: Vec<(String, notc::ParseResult)> = files
        .iter()
        .map(|(rel, _, src)| {
            (
                rel.clone(),
                notc::parse_file_with_frontends(rel, src, &frontends),
            )
        })
        .collect();
    let has_parse_errors = parsed_files.iter().any(|(_, p)| !p.diagnostics.is_empty());
    let module_refs: Vec<(&str, &notc::Module)> = parsed_files
        .iter()
        .map(|(rel, p)| (rel.as_str(), &p.program.root))
        .collect();
    let project = if has_parse_errors {
        let analysis = notc::ide::WorkspaceAnalysis::analyze_with_frontends(&pairs, &frontends);
        emit_notc_diagnostics(&files, analysis.diagnostics());
        return ExitCode::FAILURE;
    } else {
        match notc::check_parsed_modules(&module_refs) {
            Ok(project) => project,
            Err(_) => {
                let analysis =
                    notc::ide::WorkspaceAnalysis::analyze_with_frontends(&pairs, &frontends);
                emit_notc_diagnostics(&files, analysis.diagnostics());
                return ExitCode::FAILURE;
            }
        }
    };
    let block = match project.eval_document(&entry_rel) {
        Ok(block) => block,
        Err(err) => {
            eprintln!("error evaluating `{}`: {err:?}", file.display());
            return ExitCode::FAILURE;
        }
    };
    let body = notist_ssg::render_standalone_html(&block);
    let html_doc = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<style>{}</style>\n</head>\n<body>\n<main>{body}</main>\n</body>\n</html>\n",
        notist_html::highlight::CSS
    );
    if let Err(err) = std::fs::create_dir_all(out_dir) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }
    let out_file = out_dir.join("index.html");
    if let Err(err) = std::fs::write(&out_file, html_doc) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }
    println!("{}", out_file.display());
    ExitCode::SUCCESS
}

#[cfg(not(target_arch = "wasm32"))]
fn is_module(file: &Path) -> bool {
    file.extension().is_some_and(|ext| ext == "notc")
}
#[cfg(not(target_arch = "wasm32"))]
fn load_vault(config: Option<&Path>) -> notist::Vault {
    let mut vault = notist::Vault::open(".");
    if let Some(config) = config {
        vault = vault.with_config(config);
    }
    vault
}
#[cfg(not(target_arch = "wasm32"))]
fn emit_vault_error(error: notist::VaultError) -> ExitCode {
    match error {
        notist::VaultError::Environment(errors) => {
            for error in errors {
                emit_source(
                    &error.path,
                    &error.source,
                    &[error.diagnostic],
                    Severity::Error,
                );
            }
        }
        error => eprintln!("{error}"),
    }
    ExitCode::FAILURE
}
#[cfg(not(target_arch = "wasm32"))]
fn emit_source_diagnostics(diagnostics: &[notist::SourceDiagnostic], severity: Severity) {
    for diagnostic in diagnostics {
        emit_source(
            &diagnostic.path,
            &diagnostic.source,
            std::slice::from_ref(&diagnostic.diagnostic),
            severity,
        );
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn emit_source(path: &Path, source: &str, diagnostics: &[notist::Diagnostic], severity: Severity) {
    let writer = StandardStream::stderr(stderr_color_choice());
    write_source(&mut writer.lock(), path, source, diagnostics, severity);
}

#[cfg(not(target_arch = "wasm32"))]
fn stderr_color_choice() -> ColorChoice {
    use std::io::IsTerminal;
    if std::io::stderr().is_terminal() {
        ColorChoice::Auto
    } else {
        ColorChoice::Never
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn write_source(
    writer: &mut impl codespan_reporting::term::termcolor::WriteColor,
    path: &Path,
    source: &str,
    diagnostics: &[notist::Diagnostic],
    severity: Severity,
) {
    let files = SimpleFile::new(path.display().to_string(), source);
    for error in diagnostics {
        let diagnostic = Diagnostic::new(severity)
            .with_code(error.phase.to_string())
            .with_message(&error.message)
            .with_labels(vec![Label::primary(
                (),
                usize::from(error.span.start())..usize::from(error.span.end()),
            )]);
        let _ = term::emit(writer, &Config::default(), &files, &diagnostic);
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(dead_code)]
fn emit_diagnostics(path: &Path, source: &str, diagnostics: &[notist::Diagnostic]) {
    for diagnostic in diagnostics {
        let located = notist::SourceDiagnostic::locate(path, source, diagnostic.clone());
        emit_source(
            &located.path,
            &located.source,
            std::slice::from_ref(&located.diagnostic),
            Severity::Error,
        );
    }
}
