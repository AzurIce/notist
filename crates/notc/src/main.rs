//! Standalone `notc` compiler CLI (analogous to `rustc` for `.not` modules and `.md` documents).
//!
//! Commands:
//! - `notc check <file-or-dir>` — parse and type-check a `.not` module, `.md` document, or directory
//! - `notc eval <file> [--entry <name>]` — type-check and evaluate a `.md` document or `.not` entry function
//! - `notc ast <file>` — print the parsed frontend-agnostic AST (`notc::Module`)

use std::collections::{HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use notc::{DataHost, Decl, JsonValue, check_parsed_modules, ide, parse_file};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || matches!(args[0].as_str(), "-h" | "--help" | "help") {
        print_usage();
        return ExitCode::SUCCESS;
    }
    if matches!(args[0].as_str(), "-V" | "--version") {
        println!("notc {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }

    match args[0].as_str() {
        "check" => {
            let Some(path) = args.get(1) else {
                eprintln!("error: missing path for `notc check <file-or-dir>`");
                return ExitCode::FAILURE;
            };
            cmd_check(Path::new(path))
        }
        "eval" => {
            let Some(path) = args.get(1) else {
                eprintln!("error: missing file for `notc eval <file> [--entry <name>]`");
                return ExitCode::FAILURE;
            };
            let mut entry = "main".to_owned();
            let mut i = 2;
            while i < args.len() {
                if args[i] == "--entry"
                    && let Some(name) = args.get(i + 1)
                {
                    entry = name.clone();
                    i += 2;
                } else {
                    eprintln!("error: unexpected argument `{}`", args[i]);
                    return ExitCode::FAILURE;
                }
            }
            cmd_eval(Path::new(path), &entry)
        }
        "ast" => {
            let Some(path) = args.get(1) else {
                eprintln!("error: missing file for `notc ast <file>`");
                return ExitCode::FAILURE;
            };
            cmd_ast(Path::new(path))
        }
        other => {
            eprintln!("error: unknown subcommand `{other}` (run `notc --help`)");
            ExitCode::FAILURE
        }
    }
}

fn print_usage() {
    println!(
        "\
notc — Notist compiler for .not modules and .md documents

USAGE:
    notc check <file-or-dir>
    notc eval <file> [--entry <name>]
    notc ast <file>
    notc --version
    notc --help"
    );
}

fn normalize_fs(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(comp.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn common_root_dir(paths: &[PathBuf]) -> PathBuf {
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

fn to_virtual_path(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let mut parts = Vec::new();
    for comp in rel.components() {
        if let Component::Normal(seg) = comp {
            parts.push(seg.to_string_lossy().into_owned());
        }
    }
    parts.join("/")
}

struct LoadedGraph {
    vroot: PathBuf,
    entry_rel: String,
    files: Vec<(String, PathBuf, String)>,
}

fn load_graph(entry: &Path) -> Result<LoadedGraph, ExitCode> {
    let abs = std::fs::canonicalize(entry).unwrap_or_else(|_| {
        if entry.is_absolute() {
            entry.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| normalize_fs(&cwd.join(entry)))
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
            let norm = normalize_fs(&fs_path);
            if let Ok(src) = std::fs::read_to_string(&norm)
                && visited.insert(norm.clone())
            {
                queue.push_back(norm.clone());
                raw_files.push((norm, src));
            }
        }
    } else {
        let norm = normalize_fs(&abs);
        let src = match std::fs::read_to_string(&norm) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("{}: {err}", entry.display());
                return Err(ExitCode::FAILURE);
            }
        };
        visited.insert(norm.clone());
        queue.push_back(norm.clone());
        raw_files.push((norm, src));
    }

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
        let parsed = parse_file(&hint, &src);
        let parent = fs_path.parent().unwrap_or(Path::new("."));
        for decl in &parsed.program.root.declarations {
            if let Decl::Import(import) = decl {
                let target_fs = normalize_fs(&parent.join(&import.path));
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
        .map(|(fs_path, src)| (to_virtual_path(&vroot, &fs_path), fs_path, src))
        .collect();
    let entry_rel = files.first().map(|(r, _, _)| r.clone()).unwrap_or_default();

    Ok(LoadedGraph {
        vroot,
        entry_rel,
        files,
    })
}

struct FsDataHost {
    vroot: PathBuf,
}

impl FsDataHost {
    fn read_regular(&self, from_doc: &str, rel_path: &str, kind: &str) -> Result<String, String> {
        let target_fs = if rel_path.starts_with('/') {
            normalize_fs(&self.vroot.join(rel_path.trim_start_matches('/')))
        } else {
            let doc_fs = self.vroot.join(from_doc);
            let parent = doc_fs.parent().unwrap_or(&self.vroot);
            normalize_fs(&parent.join(rel_path))
        };
        let meta = std::fs::symlink_metadata(&target_fs)
            .map_err(|err| format!("failed to read {kind} `{rel_path}`: {err}"))?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(format!("{kind} `{rel_path}` must be a regular file"));
        }
        std::fs::read_to_string(&target_fs)
            .map_err(|err| format!("failed to read {kind} `{rel_path}`: {err}"))
    }
}

impl DataHost for FsDataHost {
    fn read_text(&self, from_doc: &str, rel_path: &str) -> Result<String, String> {
        self.read_regular(from_doc, rel_path, "text")
    }

    fn read_json(&self, from_doc: &str, rel_path: &str) -> Result<JsonValue, String> {
        let raw = self.read_regular(from_doc, rel_path, "JSON")?;
        JsonValue::parse(&raw).map_err(|err| format!("invalid JSON in `{rel_path}`: {err}"))
    }
}

fn line_col(source: &str, byte_offset: u32) -> (usize, usize) {
    let off = (byte_offset as usize).min(source.len());
    let prefix = &source[..off];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
    let col = prefix.rsplit('\n').next().map_or(1, |s| s.chars().count() + 1);
    (line, col)
}

fn emit_diagnostics(files: &[(String, PathBuf, String)], diags: &[ide::IdeDiagnostic]) {
    for d in diags {
        let phase = match d.phase {
            ide::DiagnosticPhase::Parse => "syntax",
            ide::DiagnosticPhase::Type => "type",
        };
        if let Some((_, fs_path, src)) = files.iter().find(|(rel, _, _)| rel == &d.module) {
            let (line, col) = line_col(src, d.span.start);
            eprintln!(
                "{}:{}:{}: error[{phase}]: {}",
                fs_path.display(),
                line,
                col,
                d.message
            );
        } else {
            eprintln!("{}: error[{phase}]: {}", d.module, d.message);
        }
    }
}

fn cmd_ast(file: &Path) -> ExitCode {
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("{}: {err}", file.display());
            return ExitCode::FAILURE;
        }
    };
    let hint = file.to_string_lossy();
    let parsed = parse_file(&hint, &src);
    println!("{:#?}", parsed.program.root);
    if parsed.diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        for d in &parsed.diagnostics {
            let (line, col) = line_col(&src, d.span.start);
            eprintln!("{}:{}:{}: error[syntax]: {}", file.display(), line, col, d.message);
        }
        ExitCode::FAILURE
    }
}

fn cmd_check(path: &Path) -> ExitCode {
    let Ok(LoadedGraph { files, .. }) = load_graph(path) else {
        return ExitCode::FAILURE;
    };
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, _, src)| (rel.as_str(), src.as_str()))
        .collect();
    let analysis = ide::WorkspaceAnalysis::analyze(&pairs);
    let diags = analysis.diagnostics();
    if diags.is_empty() {
        ExitCode::SUCCESS
    } else {
        emit_diagnostics(&files, diags);
        ExitCode::FAILURE
    }
}

fn cmd_eval(file: &Path, entry: &str) -> ExitCode {
    let Ok(LoadedGraph {
        vroot,
        entry_rel,
        files,
    }) = load_graph(file)
    else {
        return ExitCode::FAILURE;
    };
    let parsed: Vec<(String, notc::ParseResult)> = files
        .iter()
        .map(|(rel, _, src)| (rel.clone(), parse_file(rel, src)))
        .collect();
    let pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, _, src)| (rel.as_str(), src.as_str()))
        .collect();
    if parsed.iter().any(|(_, p)| !p.diagnostics.is_empty()) {
        let analysis = ide::WorkspaceAnalysis::analyze(&pairs);
        emit_diagnostics(&files, analysis.diagnostics());
        return ExitCode::FAILURE;
    }
    let module_refs: Vec<(&str, &notc::Module)> = parsed
        .iter()
        .map(|(rel, p)| (rel.as_str(), &p.program.root))
        .collect();
    let project = match check_parsed_modules(&module_refs) {
        Ok(p) => p,
        Err(_) => {
            let analysis = ide::WorkspaceAnalysis::analyze(&pairs);
            emit_diagnostics(&files, analysis.diagnostics());
            return ExitCode::FAILURE;
        }
    };
    let host = FsDataHost { vroot };
    let is_code_module = file.extension().is_some_and(|e| e == "not");
    if is_code_module {
        match project.run_with_host(&entry_rel, entry, &host) {
            Ok(val) => {
                println!("{val:#?}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error evaluating `{}::{entry}`: {err:?}", file.display());
                ExitCode::FAILURE
            }
        }
    } else {
        match project.eval_document_with_host(&entry_rel, &host) {
            Ok(block) => {
                println!("{block}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("error evaluating `{}`: {err:?}", file.display());
                ExitCode::FAILURE
            }
        }
    }
}
