//! HTTP server for the same publication artifact produced by static builds.
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    net::TcpListener,
    path::{Component, Path, PathBuf},
    sync::{Arc, RwLock, mpsc},
    thread,
    time::{Duration, Instant},
};
use tiny_http::{Header, Method, Request, Response, Server};

mod report;
mod terminal;
mod timings;
mod watch;
use report::Report;
use terminal::Terminal;

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
fn assemble(
    target: &crate::package::Package,
    executor: &Arc<notist::Executor>,
) -> Result<Build, (Error, BTreeSet<PathBuf>)> {
    crate::ssg::assemble(target, true, executor)
        .map(|b| Build {
            artifact: BuildOutput {
                pages: b.artifact.pages.len(),
                home: b
                    .artifact
                    .pages
                    .iter()
                    .find(|p| p.route.url == "/")
                    .or_else(|| b.artifact.pages.first())
                    .map(|p| p.route.url.clone()),
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
                    notist_ssg::Error::Diagnostics(d) => Error::Diagnostics(d),
                    e => Error::Message(e.to_string()),
                },
                inputs,
            )
        })
}

struct Published {
    site: Option<Arc<BuildOutput>>,
    revision: u64,
    event: u64,
    report: Report,
    timings: Option<timings::History>,
}
type State = Arc<RwLock<Published>>;
const CLIENT: &str = include_str!("serve.js");

pub(crate) fn serve_package(
    target: crate::package::Package,
    listen: &crate::ListenOptions,
    no_clear_screen: bool,
    verbose: bool,
    jobs: Option<std::num::NonZeroUsize>,
    timings: bool,
) -> std::process::ExitCode {
    match run(target, listen, no_clear_screen, verbose, jobs, timings) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn bind(listen: &crate::ListenOptions) -> std::io::Result<TcpListener> {
    if let Some(address) = &listen.address {
        return TcpListener::bind(address);
    }
    if let Some(port) = listen.port {
        return TcpListener::bind((listen.host.as_str(), port));
    }
    bind_default(&listen.host, 8000)
}

fn bind_default(host: &str, preferred_port: u16) -> std::io::Result<TcpListener> {
    match TcpListener::bind((host, preferred_port)) {
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => TcpListener::bind((host, 0)),
        result => result,
    }
}

#[cfg(test)]
mod listen_tests {
    use super::*;

    #[test]
    fn default_port_falls_back_if_occupied() {
        let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = occupied.local_addr().unwrap().port();
        let listener = bind_default("127.0.0.1", port).unwrap();
        assert_ne!(listener.local_addr().unwrap().port(), port);
    }

    #[test]
    fn default_port_is_preferred_when_available() {
        let listener = bind_default("127.0.0.1", 0).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let listener = bind_default("127.0.0.1", port).unwrap();
        assert_eq!(listener.local_addr().unwrap().port(), port);
    }
}

fn run(
    target: crate::package::Package,
    listen: &crate::ListenOptions,
    no_clear_screen: bool,
    verbose: bool,
    jobs: Option<std::num::NonZeroUsize>,
    timings: bool,
) -> Result<(), Error> {
    // One pool for the initial build and every subsequent watcher rebuild.
    let executor = crate::ssg::executor(jobs).map_err(Error::Message)?;
    // Keep the bound listener: probing and then rebinding would race other processes.
    let listener = bind(listen).map_err(|error| Error::Message(error.to_string()))?;
    let server =
        Server::from_listener(listener, None).map_err(|error| Error::Message(error.to_string()))?;
    let terminal = Terminal::new(
        format!("http://{}/", server.server_addr()),
        no_clear_screen || verbose,
    );
    let state = Arc::new(RwLock::new(Published {
        site: None,
        revision: 0,
        event: 0,
        report: Report::default(),
        timings: timings.then(timings::History::new),
    }));
    let (sender, receiver) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = sender.send((Instant::now(), event));
    })
    .map_err(|error| Error::Message(error.to_string()))?;
    let mut observed = BTreeSet::new();
    let mut output = None;
    let mut watches = watch::Watches::default();
    let mut plan = watch::Plan::new(&target, &observed, None);
    watches.sync(&mut watcher, &plan)?;
    rebuild(
        &state,
        &mut observed,
        &mut output,
        &terminal,
        &target,
        &executor,
        None,
    );
    plan = watch::Plan::new(&target, &observed, output.as_deref());
    watches.sync(&mut watcher, &plan)?;
    let shared = state.clone();
    thread::spawn(move || {
        let mut watcher = watcher;
        while let Ok((received, event)) = receiver.recv() {
            watches.observe(&event);
            let relevant = |event: &Result<notify::Event, notify::Error>| match event {
                Ok(event) => {
                    !matches!(event.kind, notify::EventKind::Access(_))
                        && event.paths.iter().any(|path| plan.relevant(path))
                }
                Err(_) => true,
            };
            if !relevant(&event) {
                continue;
            }
            let mut changes = BTreeSet::new();
            if let Ok(event) = &event {
                changes.extend(event.paths.iter().cloned());
            }
            let deadline = Instant::now() + Duration::from_millis(200);
            while Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                match receiver.recv_timeout(remaining) {
                    Ok((_, event)) => {
                        if relevant(&event)
                            && let Ok(event) = &event
                        {
                            changes.extend(event.paths.iter().cloned());
                        }
                        watches.observe(&event);
                    }
                    Err(_) => break,
                }
            }
            crate::progress::observe(verbose, || {
                rebuild(
                    &shared,
                    &mut observed,
                    &mut output,
                    &terminal,
                    &target,
                    &executor,
                    Some((received, changes)),
                );
            });
            plan = watch::Plan::new(&target, &observed, output.as_deref());
            if let Err(error) = watches.sync(&mut watcher, &plan) {
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
    state: &State,
    observed: &mut BTreeSet<PathBuf>,
    output: &mut Option<PathBuf>,
    terminal: &Terminal,
    target: &crate::package::Package,
    executor: &Arc<notist::Executor>,
    trigger: Option<(Instant, BTreeSet<PathBuf>)>,
) {
    let (previous, has_preview) = {
        let mut published = state.write().unwrap();
        published.report.begin();
        published.event += 1;
        (published.report.clone(), published.site.is_some())
    };
    terminal.render(&previous, has_preview);
    let started = Instant::now();
    let enabled = state.read().unwrap().timings.is_some();
    let mut build = || {
        let span =
            notist::progress::Span::stage("host.rebuild", || format!("Rebuild {}", target.name));
        let result = assemble(target, executor);
        let mut published = state.write().unwrap();
        published.event += 1;
        match result {
            Ok(result) => {
                *observed = result.observed;
                *output = Some(result.output);
                notist::progress::measure(
                    "host.publish",
                    || "Publish preview".into(),
                    || {
                        published.site = Some(Arc::new(result.artifact));
                        published.revision += 1;
                    },
                );
                published.report =
                    previous.success(published.site.as_ref().unwrap(), started.elapsed());
                span.finish();
            }
            Err((error, inputs)) => {
                *observed = inputs;
                published.report = previous.failure(&error, started.elapsed());
            }
        }
        (published.report.clone(), published.site.is_some())
    };
    let ((report, has_preview), timing) = if enabled {
        let (result, mut timing) = notist::progress::timings::capture(build);
        timing
            .attributes
            .insert("package".into(), target.name.clone());
        timing.attributes.insert(
            "scope".into(),
            "full rebuild; cached module results may be reused".into(),
        );
        timing
            .attributes
            .insert("pages".into(), result.0.pages.to_string());
        timing.attributes.insert(
            "status".into(),
            serde_json::to_value(result.0.status)
                .unwrap()
                .as_str()
                .unwrap()
                .into(),
        );
        timing.attributes.insert(
            "trigger".into(),
            if trigger.is_some() {
                "file change / watcher notification"
            } else {
                "initial build"
            }
            .into(),
        );
        if let Some((received, changes)) = trigger {
            timing.attributes.insert(
                "update_latency_ms".into(),
                format!("{:.3}", received.elapsed().as_secs_f64() * 1000.0),
            );
            timing.attributes.insert(
                "changed_files".into(),
                changes
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        (result, Some(timing))
    } else {
        (build(), None)
    };
    if let Some(timing) = timing {
        let mut published = state.write().unwrap();
        let history = published.timings.as_mut().unwrap();
        timings::retain(history, report.build, timing);
    }
    terminal.render(&report, has_preview);
    if enabled {
        terminal.timings(report.build);
    }
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
    if path == Path::new("_notist/timings") || path.starts_with("_notist/timings") {
        timings::respond(request, &state);
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
