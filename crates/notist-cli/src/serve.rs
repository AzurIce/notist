//! HTTP server for the same publication artifact produced by static builds.
use crate::book;
use notify::{RecursiveMode, Watcher};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Arc, RwLock, mpsc},
    thread,
    time::{Duration, Instant},
};
use tiny_http::{Header, Method, Request, Response, Server};

mod report;
mod terminal;
use report::Report;
use terminal::Terminal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Book,
    Site,
}

struct BuildOutput {
    files: BTreeMap<PathBuf, Vec<u8>>,
    pages: usize,
    home: Option<String>,
    diagnostics: Vec<notist::SourceDiagnostic>,
    warnings: Vec<notist::SourceDiagnostic>,
}
enum Error {
    Diagnostics(Vec<notist::SourceDiagnostic>),
    Message(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(s) => f.write_str(s),
            Self::Diagnostics(d) => {
                for d in d {
                    writeln!(f, "{}: {}", d.path.display(), d.diagnostic.message)?;
                }
                Ok(())
            }
        }
    }
}
struct Build {
    artifact: BuildOutput,
    output: PathBuf,
    observed: BTreeSet<PathBuf>,
}
struct Assembly {
    kind: Kind,
    target: Option<crate::workspace::Target>,
}
impl Kind {
    fn assemble(
        self,
        root: &Path,
        config: Option<&Path>,
        target: Option<&crate::workspace::Target>,
    ) -> Result<Build, (Error, BTreeSet<PathBuf>)> {
        match self {
            Self::Book => book::assemble(root, config, None, true, target)
                .map(|b| Build {
                    artifact: BuildOutput {
                        pages: b.artifact.pages.len(),
                        home: b.artifact.pages.first().map(|p| p.route.url.clone()),
                        files: b.artifact.files,
                        diagnostics: b.artifact.diagnostics,
                        warnings: b.artifact.warnings,
                    },
                    output: b.output,
                    observed: b.observed,
                })
                .map_err(|(e, inputs)| {
                    (
                        match e {
                            notist_book::Error::Diagnostics(d) => Error::Diagnostics(d),
                            e => Error::Message(e.to_string()),
                        },
                        inputs,
                    )
                }),
            Self::Site => crate::ssg::assemble(root, config, None, true, target)
                .map(|b| Build {
                    artifact: BuildOutput {
                        pages: b.artifact.pages.len(),
                        home: None,
                        files: b.artifact.files,
                        diagnostics: vec![],
                        warnings: b.artifact.warnings,
                    },
                    output: b.output,
                    observed: b.observed,
                })
                .map_err(|(e, inputs)| {
                    (
                        match e {
                            notist_ssg::Error::Diagnostics(d) => Error::Diagnostics(d),
                            e => Error::Message(e.to_string()),
                        },
                        inputs,
                    )
                }),
        }
    }
}

struct Published {
    site: Option<Arc<BuildOutput>>,
    revision: u64,
    event: u64,
    report: Report,
}
type State = Arc<RwLock<Published>>;
const CLIENT: &str = include_str!("serve.js");

pub fn serve(
    root: &Path,
    config: Option<&Path>,
    address: &str,
    no_clear_screen: bool,
    kind: Kind,
) -> std::process::ExitCode {
    serve_inner(root, config, address, no_clear_screen, kind, None)
}
pub(crate) fn serve_target(
    target: crate::workspace::Target,
    address: &str,
    no_clear_screen: bool,
) -> std::process::ExitCode {
    let root = target.root.clone();
    serve_inner(
        &root,
        None,
        address,
        no_clear_screen,
        target.kind,
        Some(target),
    )
}
fn serve_inner(
    root: &Path,
    config: Option<&Path>,
    address: &str,
    no_clear_screen: bool,
    kind: Kind,
    target: Option<crate::workspace::Target>,
) -> std::process::ExitCode {
    match run(root, config, address, no_clear_screen, kind, target) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}
fn run(
    root: &Path,
    config: Option<&Path>,
    address: &str,
    no_clear_screen: bool,
    kind: Kind,
    target: Option<crate::workspace::Target>,
) -> Result<(), Error> {
    let root = std::fs::canonicalize(root).map_err(|error| Error::Message(error.to_string()))?;
    let config = config
        .map(|path| notist::resources::normalize(&std::env::current_dir().unwrap().join(path)));
    let server = Server::http(address).map_err(|error| Error::Message(error.to_string()))?;
    let terminal = Terminal::new(format!("http://{}/", server.server_addr()), no_clear_screen);
    let state = Arc::new(RwLock::new(Published {
        site: None,
        revision: 0,
        event: 0,
        report: Report::default(),
    }));
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = sender.send(event);
    })
    .map_err(|error| Error::Message(error.to_string()))?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|error| Error::Message(error.to_string()))?;
    let mut observed = BTreeSet::new();
    let mut watched = BTreeMap::new();
    let mut output = None;
    let assembly = Assembly { kind, target };
    rebuild(
        &root,
        config.as_deref(),
        &state,
        &mut observed,
        &mut output,
        &terminal,
        &assembly,
    );
    watch_external(&mut watcher, &root, &observed, &mut watched)?;
    let shared = state.clone();
    let watcher_root = root.clone();
    thread::spawn(move || {
        let mut watcher = watcher;
        while let Ok(event) = receiver.recv() {
            let relevant = |event: &Result<notify::Event, notify::Error>| match event {
                Ok(event) => {
                    !matches!(event.kind, notify::EventKind::Access(_))
                        && event.paths.iter().any(|path| {
                            if output
                                .as_ref()
                                .is_some_and(|output| path.starts_with(output))
                            {
                                return false;
                            }
                            if let Ok(relative) = path.strip_prefix(&watcher_root) {
                                !relative.iter().any(|part| {
                                    part.to_string_lossy().starts_with('.')
                                        || part == "target"
                                        || part == "node_modules"
                                })
                            } else {
                                observed.iter().any(|input| {
                                    input.starts_with(path)
                                        || (input.is_dir() && path.starts_with(input))
                                })
                            }
                        })
                }
                Err(_) => true,
            };
            if !relevant(&event) {
                continue;
            }
            let deadline = Instant::now() + Duration::from_millis(200);
            while Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if receiver.recv_timeout(remaining).is_err() {
                    break;
                }
            }
            rebuild(
                &watcher_root,
                config.as_deref(),
                &shared,
                &mut observed,
                &mut output,
                &terminal,
                &assembly,
            );
            if let Err(error) = watch_external(&mut watcher, &watcher_root, &observed, &mut watched)
            {
                eprintln!("{error}");
            }
        }
    });
    println!("serve http://{}/", server.server_addr());
    std::io::stdout()
        .flush()
        .map_err(|error| Error::Message(error.to_string()))?;
    for request in server.incoming_requests() {
        let state = state.clone();
        thread::spawn(move || respond(request, state));
    }
    Ok(())
}
fn rebuild(
    root: &Path,
    config: Option<&Path>,
    state: &State,
    observed: &mut BTreeSet<PathBuf>,
    output: &mut Option<PathBuf>,
    terminal: &Terminal,
    assembly: &Assembly,
) {
    let (previous, has_preview) = {
        let mut published = state.write().unwrap();
        published.report.begin();
        published.event += 1;
        (published.report.clone(), published.site.is_some())
    };
    terminal.render(&previous, has_preview);
    let started = Instant::now();
    let result = assembly
        .kind
        .assemble(root, config, assembly.target.as_ref());
    let mut published = state.write().unwrap();
    published.event += 1;
    match result {
        Ok(result) => {
            *observed = result.observed;
            *output = Some(result.output);
            published.report = previous.success(&result.artifact, started.elapsed());
            published.site = Some(Arc::new(result.artifact));
            published.revision += 1;
        }
        Err((error, inputs)) => {
            *observed = inputs;
            published.report = previous.failure(&error, started.elapsed());
        }
    }
    let report = published.report.clone();
    let has_preview = published.site.is_some();
    drop(published);
    terminal.render(&report, has_preview);
}
fn watch_external(
    watcher: &mut impl Watcher,
    root: &Path,
    observed: &BTreeSet<PathBuf>,
    watched: &mut BTreeMap<PathBuf, bool>,
) -> Result<(), Error> {
    // Backends drop directory watches when the directory is deleted. Permit
    // registration again if an external input directory is later recreated.
    watched.retain(|directory, _| directory.is_dir());
    for input in observed.iter().filter(|path| !path.starts_with(root)) {
        let recursive = input.is_dir();
        let mut directory = if recursive {
            input.as_path()
        } else {
            input.parent().unwrap()
        };
        while !directory.is_dir() {
            directory = directory.parent().unwrap();
        }
        // File dependencies only need their parent. Recursing a fallback
        // ancestor (possibly /) would watch unrelated trees and delay startup.
        if watched
            .get(directory)
            .is_none_or(|previous| recursive && !previous)
        {
            if watched.contains_key(directory) {
                watcher
                    .unwatch(directory)
                    .map_err(|error| Error::Message(error.to_string()))?;
            }
            watcher
                .watch(
                    directory,
                    if recursive {
                        RecursiveMode::Recursive
                    } else {
                        RecursiveMode::NonRecursive
                    },
                )
                .map_err(|error| Error::Message(error.to_string()))?;
            watched.insert(directory.into(), recursive);
        }
    }
    Ok(())
}
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).unwrap()
}
fn simple(request: Request, code: u16, bytes: Vec<u8>, content_type: &str) {
    let response = Response::from_data(bytes)
        .with_status_code(code)
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store"));
    let _ = request.respond(response);
}
fn respond(request: Request, state: State) {
    if request.method() != &Method::Get && request.method() != &Method::Head {
        simple(
            request,
            405,
            b"Method not allowed".to_vec(),
            "text/plain; charset=utf-8",
        );
        return;
    }
    let raw = request.url().split('?').next().unwrap();
    let Ok(decoded) = percent_encoding::percent_decode_str(raw).decode_utf8() else {
        simple(request, 400, b"Invalid URL".to_vec(), "text/plain");
        return;
    };
    let path = Path::new(decoded.trim_start_matches('/'));
    if decoded.contains('\\')
        || decoded.contains('\0')
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        simple(request, 400, b"Invalid URL".to_vec(), "text/plain");
        return;
    }
    if path == Path::new("_notist/serve/events") && request.method() == &Method::Get {
        events(request, state);
        return;
    }
    if path == Path::new("_notist/serve/client.js") {
        simple(
            request,
            200,
            CLIENT.as_bytes().to_vec(),
            "text/javascript; charset=utf-8",
        );
        return;
    }
    let published = state.read().unwrap();
    let Some(site) = &published.site else {
        let revision = published.revision;
        drop(published);
        let html = format!(
            "<!doctype html><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Notist serve</title><h1>No preview available yet</h1><p>Fix the errors below. This page reloads automatically when a preview is available.</p><script type=\"module\" src=\"{}\"></script>",
            serve_url(
                &if decoded.ends_with('/') {
                    path.join("index.html")
                } else {
                    path.to_path_buf()
                },
                revision
            )
        );
        simple(request, 503, html.into_bytes(), "text/html; charset=utf-8");
        return;
    };
    let mut file = path.to_path_buf();
    if decoded.ends_with('/') {
        file.push("index.html");
    } else if site.files.contains_key(&file.join("index.html")) {
        let url = format!("/{}/", notist::publication::url::encode_path(&file));
        let _ = request.respond(Response::empty(308).with_header(header("Location", &url)));
        return;
    }
    if file == Path::new("index.html")
        && !site.files.contains_key(&file)
        && let Some(home) = &site.home
    {
        let _ = request.respond(Response::empty(302).with_header(header("Location", home)));
        return;
    }
    let (bytes, status, content_type) = match site.files.get(&file) {
        Some(bytes) => (bytes, 200, mime(&file)),
        None => match site.files.get(Path::new("404.html")) {
            Some(bytes) => (bytes, 404, "text/html; charset=utf-8"),
            None => {
                drop(published);
                simple(
                    request,
                    404,
                    b"Page not found".to_vec(),
                    "text/plain; charset=utf-8",
                );
                return;
            }
        },
    };
    let mut bytes = bytes.clone();
    let revision = published.revision;
    drop(published);
    if content_type == "text/html; charset=utf-8" {
        let html = String::from_utf8_lossy(&bytes);
        let script = format!(
            "<script type=\"module\" src=\"{}\"></script>",
            serve_url(&file, revision)
        );
        bytes = if let Some(offset) = html.rfind("</body>") {
            format!("{}{}{}", &html[..offset], script, &html[offset..]).into_bytes()
        } else {
            format!("{html}{script}").into_bytes()
        };
    }
    simple(request, status, bytes, content_type);
}
fn serve_url(file: &Path, revision: u64) -> String {
    format!(
        "{}{}_notist/serve/client.js?v={revision}",
        "../".repeat(file.parent().unwrap_or(Path::new("")).components().count()),
        if file
            .parent()
            .unwrap_or(Path::new(""))
            .as_os_str()
            .is_empty()
        {
            "./"
        } else {
            ""
        }
    )
}
fn mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
    {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "txt" => "text/plain; charset=utf-8",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
fn events(request: Request, state: State) {
    let mut writer = request.into_writer();
    if writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n").is_err() { return; }
    let mut previous = None;
    let mut heartbeat = Instant::now();
    loop {
        let published = state.read().unwrap();
        let event = published.event;
        let message = if previous != Some(event) {
            previous = Some(event);
            let data = serde_json::json!({
                "revision": published.revision,
                "report": published.report,
                "summary": published.report.summary(published.site.is_some()),
            });
            Some(format!("event: update\ndata: {data}\n\n"))
        } else if heartbeat.elapsed() > Duration::from_secs(10) {
            Some(": heartbeat\n\n".into())
        } else {
            None
        };
        drop(published);
        if let Some(message) = message {
            heartbeat = Instant::now();
            if writer
                .write_all(message.as_bytes())
                .and_then(|_| writer.flush())
                .is_err()
            {
                return;
            }
        }
        thread::sleep(Duration::from_millis(250));
    }
}
