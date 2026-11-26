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
    /// Check a document/directory, or dry-run check the current Notist.toml site
    Check {
        /// File or directory to check (omit to dry-run check the current package/workspace site)
        file: Option<PathBuf>,
        /// Check every workspace package that declares a site
        #[arg(long, conflicts_with = "file")]
        workspace: bool,
        /// Select workspace packages by name (repeatable)
        #[arg(short = 'p', long = "package", conflicts_with = "file")]
        packages: Vec<String>,
    },
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
        Command::Check {
            file,
            workspace,
            packages,
        } => match file {
            Some(file) => check(&file, config),
            None => package::check(config, workspace, &packages, cli.jobs),
        },
        Command::Cst { file } => cmd_cst(&file),
        Command::Core { file } => cmd_core(&file, config, evaluate),
        Command::Json { file } => cmd_json(&file, config, evaluate),
        Command::Query { file, selector } => cmd_query(&file, config, &selector),
        Command::Html { file, out_dir } => render_html_notc(&file, config, &out_dir),
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
fn common_root_dir(paths: &[PathBuf]) -> PathBuf {
    use std::path::Component;
    let dirs: Vec<PathBuf> = paths
        .iter()
        .map(|p| {
            if p.is_dir() {
                p.clone()
            } else {
                p.parent().unwrap_or(p).to_path_buf()
            }
        })
        .collect();
    let Some(first) = dirs.first() else {
        return PathBuf::from(".");
    };
    let mut prefix: Vec<Component<'_>> = first.components().collect();
    for dir in &dirs[1..] {
        let comps: Vec<Component<'_>> = dir.components().collect();
        let common = prefix
            .iter()
            .zip(&comps)
            .take_while(|(a, b)| a == b)
            .count();
        prefix.truncate(common);
    }
    if prefix.is_empty() {
        PathBuf::from(".")
    } else {
        prefix.into_iter().collect()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn to_virtual_path(root: &Path, path: &Path) -> String {
    use std::path::Component;
    let rel = path.strip_prefix(root).unwrap_or(path);
    let mut parts = Vec::new();
    for comp in rel.components() {
        if let Component::Normal(seg) = comp {
            parts.push(seg.to_string_lossy().into_owned());
        }
    }
    parts.join("/")
}

#[cfg(not(target_arch = "wasm32"))]
struct LoadedGraph {
    vroot: PathBuf,
    entry_rel: String,
    files: Vec<(String, PathBuf, String)>,
}

#[cfg(not(target_arch = "wasm32"))]
fn canonical_or_norm(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| notist::resources::normalize(path))
}

#[cfg(not(target_arch = "wasm32"))]
fn load_replacements_for_file(
    file: &Path,
    config: Option<&Path>,
) -> Result<Vec<notist_ssg::ReplacementRule>, ExitCode> {
    package::load_document_replacements(file, config).map_err(|err| {
        eprintln!("{err}");
        ExitCode::FAILURE
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn load_notc_graph(
    entry: &Path,
    replacements: &[notist_ssg::ReplacementRule],
) -> Result<LoadedGraph, ExitCode> {
    use std::collections::{HashSet, VecDeque};
    let abs = std::fs::canonicalize(entry).unwrap_or_else(|_| {
        if entry.is_absolute() {
            entry.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| notist::resources::normalize(&cwd.join(entry)))
                .unwrap_or_else(|_| entry.to_path_buf())
        }
    });

    let mut raw_files: Vec<(PathBuf, String)> = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::new();

    if abs.is_dir() {
        fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) {
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
                    walk_dir(&p, out);
                } else if p
                    .extension()
                    .is_some_and(|e| e == "not" || e == "md" || e == "markdown")
                {
                    out.push(p);
                }
            }
        }
        let mut discovered = Vec::new();
        walk_dir(&abs, &mut discovered);
        for fs_path in discovered {
            let norm = notist::resources::normalize(&fs_path);
            if let Ok(src) = std::fs::read_to_string(&norm)
                && visited.insert(norm.clone())
            {
                queue.push_back(norm.clone());
                raw_files.push((norm, src));
            }
        }
    } else {
        let norm = notist::resources::normalize(&abs);
        let src = read(&norm)?;
        visited.insert(norm.clone());
        queue.push_back(norm.clone());
        raw_files.push((norm, src));
    }

    // Seed `.not` modules referenced by `[[extensions]]` replacement rules.
    for rule in replacements {
        for mod_path in [&rule.from_module, &rule.to_module].into_iter().flatten() {
            let norm = canonical_or_norm(mod_path);
            if visited.insert(norm.clone()) {
                let src = read(&norm)?;
                queue.push_back(norm.clone());
                raw_files.push((norm, src));
            }
        }
    }

    // Transitively load imported `.not` modules using real normalized filesystem paths
    // so parent-relative imports like `../lib/ui.not` resolve cleanly.
    while let Some(fs_path) = queue.pop_front() {
        let Some(src) = raw_files
            .iter()
            .find(|(p, _)| p == &fs_path)
            .map(|(_, s)| s.clone())
        else {
            continue;
        };
        let hint = fs_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let parsed = notist_core::parse_file(&hint, &src);
        let parent = fs_path.parent().unwrap_or(Path::new("."));
        for decl in &parsed.program.root.declarations {
            if let notist_core::Decl::Import(import) = decl {
                let target_fs = notist::resources::normalize(&parent.join(&import.path));
                if visited.insert(target_fs.clone())
                    && let Ok(target_src) = std::fs::read_to_string(&target_fs)
                {
                    queue.push_back(target_fs.clone());
                    raw_files.push((target_fs, target_src));
                }
            }
        }
    }

    let all_paths: Vec<PathBuf> = raw_files.iter().map(|(p, _)| p.clone()).collect();
    let vroot = common_root_dir(&all_paths);
    let files: Vec<(String, PathBuf, String)> = raw_files
        .into_iter()
        .map(|(fs_path, src)| {
            let rel = to_virtual_path(&vroot, &fs_path);
            (rel, fs_path, src)
        })
        .collect();
    let entry_rel = files.first().map(|(r, _, _)| r.clone()).unwrap_or_default();

    Ok(LoadedGraph {
        vroot,
        entry_rel,
        files,
    })
}

#[cfg(not(target_arch = "wasm32"))]
struct FsDataHost {
    vroot: PathBuf,
}

#[cfg(not(target_arch = "wasm32"))]
impl FsDataHost {
    fn new(vroot: PathBuf) -> Self {
        Self { vroot }
    }

    fn read_regular_text(&self, from_doc: &str, rel_path: &str, kind: &str) -> Result<String, String> {
        let target_fs = if rel_path.starts_with('/') {
            notist::resources::normalize(&self.vroot.join(rel_path.trim_start_matches('/')))
        } else {
            let doc_fs = self.vroot.join(from_doc);
            let parent = doc_fs.parent().unwrap_or(&self.vroot);
            notist::resources::normalize(&parent.join(rel_path))
        };
        let meta = std::fs::symlink_metadata(&target_fs)
            .map_err(|err| format!("failed to read {kind} `{rel_path}`: {err}"))?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(format!(
                "{kind} `{rel_path}` must be a regular file (symlinks and directories are not allowed)"
            ));
        }
        std::fs::read_to_string(&target_fs)
            .map_err(|err| format!("failed to read {kind} `{rel_path}`: {err}"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl notist_core::DataHost for FsDataHost {
    fn read_text(&self, from_doc: &str, rel_path: &str) -> Result<String, String> {
        self.read_regular_text(from_doc, rel_path, "text")
    }

    fn read_json(&self, from_doc: &str, rel_path: &str) -> Result<notist_core::JsonValue, String> {
        let text = self.read_regular_text(from_doc, rel_path, "JSON")?;
        notist_core::JsonValue::parse(&text)
            .map_err(|err| format!("invalid JSON in `{rel_path}`: {err}"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn check_notc_project(
    files: &[(String, PathBuf, String)],
) -> Result<notist_core::CheckedProject, Vec<notist_core::ide::IdeDiagnostic>> {
    let parsed_files: Vec<(String, notist_core::ParseResult)> = files
        .iter()
        .map(|(rel, _, src)| (rel.clone(), notist_core::parse_file(rel, src)))
        .collect();
    let has_parse_errors = parsed_files.iter().any(|(_, p)| !p.diagnostics.is_empty());
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, _, src)| (rel.as_str(), src.as_str()))
        .collect();
    if has_parse_errors {
        let analysis = notist_core::ide::WorkspaceAnalysis::analyze(&pairs);
        return Err(analysis.diagnostics().to_vec());
    }
    let module_refs: Vec<(&str, &notist_core::Module)> = parsed_files
        .iter()
        .map(|(rel, p)| (rel.as_str(), &p.program.root))
        .collect();
    match notist_core::check_parsed_modules(&module_refs) {
        Ok(project) => Ok(project),
        Err(_) => {
            let analysis = notist_core::ide::WorkspaceAnalysis::analyze(&pairs);
            Err(analysis.diagnostics().to_vec())
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn apply_notc_replacements(
    project: notist_core::CheckedProject,
    vroot: &Path,
    replacements: &[notist_ssg::ReplacementRule],
) -> Result<notist_core::CheckedProject, ExitCode> {
    if replacements.is_empty() {
        return Ok(project);
    }
    let specs: Vec<notist_core::ReplacementSpec> = replacements
        .iter()
        .map(|rule| notist_core::ReplacementSpec {
            from_module: match &rule.from_module {
                Some(path) => to_virtual_path(vroot, &canonical_or_norm(path)),
                None => "notist".to_owned(),
            },
            from_name: rule.from_name.clone(),
            to_module: match &rule.to_module {
                Some(path) => to_virtual_path(vroot, &canonical_or_norm(path)),
                None => "notist".to_owned(),
            },
            to_name: rule.to_name.clone(),
        })
        .collect();
    project.with_replacements(&specs).map_err(|errs| {
        for err in &errs {
            eprintln!("{}", notist_core::format_replacement_error(err));
        }
        ExitCode::FAILURE
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn cmd_cst(file: &Path) -> ExitCode {
    let Ok(src) = read(file) else {
        return ExitCode::FAILURE;
    };
    let name = file
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let parsed = notist_core::parse_file(&name, &src);
    println!("{:#?}", parsed.program.root);
    if !parsed.diagnostics.is_empty() {
        for d in &parsed.diagnostics {
            eprintln!("error[syntax] @{}..{}: {}", d.span.start, d.span.end, d.message);
        }
    }
    ExitCode::SUCCESS
}

#[cfg(not(target_arch = "wasm32"))]
fn cmd_core(file: &Path, config: Option<&Path>, evaluate: bool) -> ExitCode {
    let Ok(replacements) = load_replacements_for_file(file, config) else {
        return ExitCode::FAILURE;
    };
    let Ok(LoadedGraph {
        vroot,
        entry_rel,
        files,
    }) = load_notc_graph(file, &replacements)
    else {
        return ExitCode::FAILURE;
    };
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, _, src)| (rel.as_str(), src.as_str()))
        .collect();
    let analysis = notist_core::ide::WorkspaceAnalysis::analyze(&pairs);
    let mut out = String::new();
    for d in analysis.diagnostics() {
        let phase = match d.phase {
            notist_core::ide::DiagnosticPhase::Parse => "syntax",
            notist_core::ide::DiagnosticPhase::Type => "type",
        };
        out.push_str(&format!(
            "error[{phase}] {}@{}..{}: {}\n",
            d.module, d.span.start, d.span.end, d.message
        ));
    }

    if is_module(file) {
        for sym in analysis.document_symbols(&entry_rel) {
            out.push_str(&format!(
                "{:?} {} : {} ({}..{})\n",
                sym.kind,
                sym.name,
                sym.detail.as_deref().unwrap_or(""),
                sym.span.start,
                sym.span.end
            ));
        }
        if evaluate
            && analysis.diagnostics().is_empty()
            && let Ok(project) = check_notc_project(&files)
        {
            let Ok(project) = apply_notc_replacements(project, &vroot, &replacements) else {
                return ExitCode::FAILURE;
            };
            let host = FsDataHost::new(vroot);
            match project.run_with_host(&entry_rel, "main", &host) {
                Ok(value) => out.push_str(&format!("main => {value:#?}\n")),
                Err(notist_core::ExecutionError::UnknownEntry(_)) => {}
                Err(err) => out.push_str(&format!("error[eval]: {err:?}\n")),
            }
        }
        print!("{out}");
        return ExitCode::SUCCESS;
    }

    if evaluate {
        match check_notc_project(&files) {
            Ok(project) => {
                let Ok(project) = apply_notc_replacements(project, &vroot, &replacements) else {
                    return ExitCode::FAILURE;
                };
                let host = FsDataHost::new(vroot);
                match project.eval_document_with_host(&entry_rel, &host) {
                    Ok(block) => out.push_str(&format!("{block}\n")),
                    Err(err) => out.push_str(&format!("error[eval]: {err:?}\n")),
                }
            }
            Err(_) => {
                // Static diagnostics were already printed above.
            }
        }
    } else if let Some((_, _, src)) = files.first() {
        let parsed = notist_core::parse_file(&entry_rel, src);
        out.push_str(&format!("{:#?}\n", parsed.program.root));
    }
    print!("{out}");
    ExitCode::SUCCESS
}

#[cfg(not(target_arch = "wasm32"))]
fn cmd_json(file: &Path, config: Option<&Path>, evaluate: bool) -> ExitCode {
    let Ok(replacements) = load_replacements_for_file(file, config) else {
        return ExitCode::FAILURE;
    };
    let Ok(LoadedGraph {
        vroot,
        entry_rel,
        files,
    }) = load_notc_graph(file, &replacements)
    else {
        return ExitCode::FAILURE;
    };
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, _, src)| (rel.as_str(), src.as_str()))
        .collect();
    let analysis = notist_core::ide::WorkspaceAnalysis::analyze(&pairs);
    let diagnostics: Vec<serde_json::Value> = analysis
        .diagnostics()
        .iter()
        .map(|d| {
            serde_json::json!({
                "module": d.module,
                "phase": match d.phase {
                    notist_core::ide::DiagnosticPhase::Parse => "syntax",
                    notist_core::ide::DiagnosticPhase::Type => "type",
                },
                "start": d.span.start,
                "end": d.span.end,
                "message": d.message,
            })
        })
        .collect();
    let symbols: Vec<serde_json::Value> = analysis
        .document_symbols(&entry_rel)
        .iter()
        .map(|s| {
            serde_json::json!({
                "name": s.name,
                "kind": format!("{:?}", s.kind).to_lowercase(),
                "detail": s.detail,
                "start": s.span.start,
                "end": s.span.end,
            })
        })
        .collect();

    let mut data = serde_json::json!({
        "module": entry_rel,
        "diagnostics": diagnostics,
        "symbols": symbols,
    });

    let entry_src = files.first().map(|(_, _, s)| s.as_str()).unwrap_or("");
    if !is_module(file) && analysis.diagnostics().is_empty() {
        if let Ok(project) = check_notc_project(&files) {
            let Ok(project) = apply_notc_replacements(project, &vroot, &replacements) else {
                return ExitCode::FAILURE;
            };
            let host = FsDataHost::new(vroot);
            match project.eval_document_with_host(&entry_rel, &host) {
                Ok(block) => {
                    data["core"] = query::serialize_block(&block, entry_src);
                }
                Err(err) => {
                    data["evaluation_error"] = serde_json::json!(format!("{err:?}"));
                }
            }
        }
    } else if is_module(file)
        && evaluate
        && analysis.diagnostics().is_empty()
        && let Ok(project) = check_notc_project(&files)
    {
        let Ok(project) = apply_notc_replacements(project, &vroot, &replacements) else {
            return ExitCode::FAILURE;
        };
        let host = FsDataHost::new(vroot);
        if let Ok(val) = project.run_with_host(&entry_rel, "main", &host) {
            data["main"] = serde_json::json!(format!("{val:?}"));
        }
    }

    println!("{data}");
    ExitCode::SUCCESS
}

#[cfg(not(target_arch = "wasm32"))]
fn cmd_query(file: &Path, config: Option<&Path>, selector: &str) -> ExitCode {
    let Ok(replacements) = load_replacements_for_file(file, config) else {
        return ExitCode::FAILURE;
    };
    let Ok(LoadedGraph {
        vroot,
        entry_rel,
        files,
    }) = load_notc_graph(file, &replacements)
    else {
        return ExitCode::FAILURE;
    };
    let project = match check_notc_project(&files) {
        Ok(project) => project,
        Err(diags) => {
            emit_notc_diagnostics(&files, &diags);
            return ExitCode::FAILURE;
        }
    };
    let Ok(project) = apply_notc_replacements(project, &vroot, &replacements) else {
        return ExitCode::FAILURE;
    };
    let host = FsDataHost::new(vroot);
    let block = match project.eval_document_with_host(&entry_rel, &host) {
        Ok(block) => block,
        Err(err) => {
            eprintln!("error evaluating `{}`: {err:?}", file.display());
            return ExitCode::FAILURE;
        }
    };
    let entry_src = files.first().map(|(_, _, s)| s.as_str()).unwrap_or("");
    match query::select_json(&block, selector, entry_src) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn emit_notc_diagnostics(
    files: &[(String, PathBuf, String)],
    diagnostics: &[notist_core::ide::IdeDiagnostic],
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
            notist_core::ide::DiagnosticPhase::Parse => "syntax",
            notist_core::ide::DiagnosticPhase::Type => "type",
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
fn check(file: &Path, config: Option<&Path>) -> ExitCode {
    let Ok(replacements) = load_replacements_for_file(file, config) else {
        return ExitCode::FAILURE;
    };
    let Ok(LoadedGraph { vroot, files, .. }) = load_notc_graph(file, &replacements) else {
        return ExitCode::FAILURE;
    };
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, _, src)| (rel.as_str(), src.as_str()))
        .collect();
    let analysis = notist_core::ide::WorkspaceAnalysis::analyze(&pairs);
    let diags = analysis.diagnostics();
    if !diags.is_empty() {
        emit_notc_diagnostics(&files, diags);
        eprintln!(
            "check: {} issue{} in {}",
            diags.len(),
            if diags.len() == 1 { "" } else { "s" },
            file.display()
        );
        return ExitCode::FAILURE;
    }
    if !replacements.is_empty() {
        let project = match check_notc_project(&files) {
            Ok(project) => project,
            Err(diags) => {
                emit_notc_diagnostics(&files, &diags);
                return ExitCode::FAILURE;
            }
        };
        if apply_notc_replacements(project, &vroot, &replacements).is_err() {
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

#[cfg(not(target_arch = "wasm32"))]
fn collect_local_images(block: &notist_core::Block, out: &mut std::collections::BTreeSet<String>) {
    fn walk_inline(inline: &notist_core::Inline, out: &mut std::collections::BTreeSet<String>) {
        match &inline.node().kind {
            notist_core::InlineKind::Image { target, .. } => {
                out.insert(target.to_string());
            }
            notist_core::InlineKind::Fragment(items) => {
                for item in items.iter() {
                    walk_inline(item, out);
                }
            }
            notist_core::InlineKind::Strong(inner)
            | notist_core::InlineKind::Emph(inner)
            | notist_core::InlineKind::Strike(inner)
            | notist_core::InlineKind::Underline(inner)
            | notist_core::InlineKind::Link { label: inner, .. } => walk_inline(inner, out),
            notist_core::InlineKind::Custom(custom) => {
                for (_, v) in custom.props.iter() {
                    match v {
                        notist_core::CustomValue::Inline(inl) => walk_inline(inl, out),
                        notist_core::CustomValue::Block(blk) => collect_local_images(blk, out),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    match &block.node().kind {
        notist_core::BlockKind::Fragment(items) | notist_core::BlockKind::List { items, .. } => {
            for item in items.iter() {
                collect_local_images(item, out);
            }
        }
        notist_core::BlockKind::Paragraph(inl) | notist_core::BlockKind::Heading { title: inl, .. } => {
            walk_inline(inl, out);
        }
        notist_core::BlockKind::Callout { body, .. }
        | notist_core::BlockKind::Quote(body)
        | notist_core::BlockKind::Div(body)
        | notist_core::BlockKind::SectionBox(body) => collect_local_images(body, out),
        notist_core::BlockKind::Section { heading, body } => {
            collect_local_images(heading, out);
            collect_local_images(body, out);
        }
        notist_core::BlockKind::Table { headers, rows, .. } => {
            for cell in headers.iter() {
                walk_inline(cell, out);
            }
            for row in rows.iter() {
                for cell in row.iter() {
                    walk_inline(cell, out);
                }
            }
        }
        notist_core::BlockKind::Custom(custom) => {
            for (_, v) in custom.props.iter() {
                match v {
                    notist_core::CustomValue::Inline(inl) => walk_inline(inl, out),
                    notist_core::CustomValue::Block(blk) => collect_local_images(blk, out),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sanitize_tag_part(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "mod".into()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn copy_dir_recursive(src_dir: &Path, dst_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst_dir)?;
    for entry in std::fs::read_dir(src_dir)? {
        let entry = entry?;
        let src_path = entry.path();
        let meta = std::fs::symlink_metadata(&src_path)?;
        if meta.file_type().is_symlink() {
            continue;
        }
        let dst_path = dst_dir.join(entry.file_name());
        if meta.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else if meta.is_file() {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn render_html_notc(file: &Path, config: Option<&Path>, out_dir: &Path) -> ExitCode {
    let Ok(replacements) = load_replacements_for_file(file, config) else {
        return ExitCode::FAILURE;
    };
    let Ok(LoadedGraph {
        vroot,
        entry_rel,
        files,
    }) = load_notc_graph(file, &replacements)
    else {
        return ExitCode::FAILURE;
    };
    let project = match check_notc_project(&files) {
        Ok(project) => project,
        Err(diags) => {
            emit_notc_diagnostics(&files, &diags);
            return ExitCode::FAILURE;
        }
    };
    let Ok(project) = apply_notc_replacements(project, &vroot, &replacements) else {
        return ExitCode::FAILURE;
    };
    let host = FsDataHost::new(vroot.clone());
    let block = match project.eval_document_with_host(&entry_rel, &host) {
        Ok(block) => block,
        Err(err) => {
            eprintln!("error evaluating `{}`: {err:?}", file.display());
            return ExitCode::FAILURE;
        }
    };

    let doc_dir = files
        .first()
        .and_then(|(_, fs_path, _)| fs_path.parent())
        .unwrap_or(Path::new("."));
    let mut images = std::collections::BTreeSet::new();
    collect_local_images(&block, &mut images);

    let mut assets_to_copy: Vec<(PathBuf, PathBuf)> = Vec::new();
    for target in images {
        let bare = target.split(['?', '#']).next().unwrap_or(&target);
        if bare.is_empty()
            || bare.starts_with('/')
            || bare.contains("://")
            || bare.starts_with("data:")
            || bare.starts_with("mailto:")
        {
            continue;
        }
        let rel = notist::resources::normalize(Path::new(bare));
        if rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir))
        {
            continue;
        }
        let src = doc_dir.join(&rel);
        let Ok(meta) = std::fs::symlink_metadata(&src) else {
            continue;
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            continue;
        }
        assets_to_copy.push((src, rel));
    }

    // Discover Web Component JS modules by convention (`<dir>/<name>.js` or `<dir>/<name>/index.js`).
    let custom_nodes = notist_ssg::collect_block_custom_nodes(&block);
    let mut web_components = std::collections::BTreeMap::new();
    let mut component_files_to_copy: Vec<(PathBuf, PathBuf, bool)> = Vec::new();
    let mut page_registrations = String::new();
    let page_script_path = PathBuf::from("_notist/page.js");
    for (module_rel, name) in &custom_nodes {
        let module_fs = vroot.join(module_rel);
        let Some(module_dir) = module_fs.parent() else {
            continue;
        };
        let single_file = module_dir.join(format!("{name}.js"));
        let dir_entry = module_dir.join(name).join("index.js");
        let (entry_out, src_path, is_dir) = if single_file.is_file() {
            let mod_rel_dir = Path::new(module_rel).parent().unwrap_or(Path::new(""));
            let out_rel = PathBuf::from("_notist/components")
                .join(mod_rel_dir)
                .join(format!("{name}.js"));
            (out_rel, single_file, false)
        } else if dir_entry.is_file() {
            let mod_rel_dir = Path::new(module_rel).parent().unwrap_or(Path::new(""));
            let dir_out = PathBuf::from("_notist/components")
                .join(mod_rel_dir)
                .join(name);
            (dir_out.join("index.js"), module_dir.join(name), true)
        } else {
            continue;
        };
        let mod_rel_dir = Path::new(module_rel).parent().unwrap_or(Path::new(""));
        let tag = format!(
            "notist-{}-{}",
            sanitize_tag_part(&mod_rel_dir.to_string_lossy()),
            sanitize_tag_part(name)
        );
        let import_rel = notist::publication::url::relative_url(&page_script_path, &entry_out, false);
        let _ = std::fmt::Write::write_fmt(
            &mut page_registrations,
            format_args!(
                "import * as _c_{idx} from \"./{import_rel}\";\nif (!customElements.get(\"{tag}\")) {{ const C = _c_{idx}.default ?? Object.values(_c_{idx}).find(v => typeof v === 'function'); if (C) customElements.define(\"{tag}\", C); }}\n",
                idx = web_components.len(),
            ),
        );
        let dst_rel = if is_dir {
            entry_out.parent().unwrap_or(Path::new("")).to_path_buf()
        } else {
            entry_out
        };
        component_files_to_copy.push((src_path, dst_rel, is_dir));
        web_components.insert((module_rel.clone(), name.clone()), tag);
    }

    let body = notist_ssg::render_standalone_html_with_components(&block, &web_components);
    let script_tag = if page_registrations.is_empty() {
        ""
    } else {
        "\n<script type=\"module\" src=\"./_notist/page.js\"></script>"
    };
    let html_doc = format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<style>{}</style>{script_tag}\n</head>\n<body>\n<main>{body}</main>\n</body>\n</html>\n",
        notist_html::highlight::CSS
    );
    if let Err(err) = std::fs::create_dir_all(out_dir) {
        eprintln!("{err}");
        return ExitCode::FAILURE;
    }
    for (src, rel) in assets_to_copy {
        let dest = out_dir.join(&rel);
        if let Some(parent) = dest.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
        if let Err(err) = std::fs::copy(&src, &dest) {
            eprintln!("{}: {err}", src.display());
            return ExitCode::FAILURE;
        }
    }
    for (src, rel, is_dir) in component_files_to_copy {
        let dest = out_dir.join(&rel);
        if is_dir {
            if let Err(err) = copy_dir_recursive(&src, &dest) {
                eprintln!("{}: {err}", src.display());
                return ExitCode::FAILURE;
            }
        } else {
            if let Some(parent) = dest.parent()
                && let Err(err) = std::fs::create_dir_all(parent)
            {
                eprintln!("{err}");
                return ExitCode::FAILURE;
            }
            if let Err(err) = std::fs::copy(&src, &dest) {
                eprintln!("{}: {err}", src.display());
                return ExitCode::FAILURE;
            }
        }
    }
    if !page_registrations.is_empty() {
        let script_out = out_dir.join(&page_script_path);
        if let Some(parent) = script_out.parent()
            && let Err(err) = std::fs::create_dir_all(parent)
        {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
        if let Err(err) = std::fs::write(&script_out, page_registrations) {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
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
    file.extension().is_some_and(|ext| ext == "not")
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
