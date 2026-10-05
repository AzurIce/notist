//! HTTP preview serves the same publication artifact produced by static builds.
use crate::site;
use notify::{RecursiveMode, Watcher};
use notist_ssg::{Error, Site};
use std::{
    collections::BTreeSet,
    io::Write,
    path::{Component, Path, PathBuf},
    sync::{Arc, RwLock, mpsc},
    thread,
    time::{Duration, Instant},
};
use tiny_http::{Header, Method, Request, Response, Server};

struct Published {
    site: Option<Arc<Site>>,
    revision: u64,
    event: u64,
    error: Option<String>,
}
type State = Arc<RwLock<Published>>;
const CLIENT: &str = include_str!("preview.js");

pub fn serve(root: &Path, config: Option<&Path>, address: &str) -> std::process::ExitCode {
    match run(root, config, address) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            site::report(error);
            std::process::ExitCode::FAILURE
        }
    }
}
fn run(root: &Path, config: Option<&Path>, address: &str) -> Result<(), Error> {
    let root = std::fs::canonicalize(root).map_err(|error| Error::Message(error.to_string()))?;
    let config = config
        .map(|path| notist::resources::normalize(&std::env::current_dir().unwrap().join(path)));
    let server = Server::http(address).map_err(|error| Error::Message(error.to_string()))?;
    let state = Arc::new(RwLock::new(Published {
        site: None,
        revision: 0,
        event: 0,
        error: None,
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
    let mut watched = BTreeSet::new();
    let mut output = root.join("target/site");
    rebuild(&root, config.as_deref(), &state, &mut observed, &mut output);
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
                            if path.starts_with(&output) {
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
            );
            if let Err(error) = watch_external(&mut watcher, &watcher_root, &observed, &mut watched)
            {
                site::report(error);
            }
        }
    });
    println!("preview http://{}/", server.server_addr());
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
    output: &mut PathBuf,
) {
    let result = site::assemble(root, config, None);
    let mut published = state.write().unwrap();
    published.event += 1;
    match result {
        Ok(result) => {
            *observed = result.observed;
            *output = result.output;
            published.site = Some(Arc::new(result.site));
            published.revision += 1;
            published.error = None;
        }
        Err((error, inputs)) => {
            *observed = inputs;
            published.error = Some(error.to_string());
            site::report(error);
        }
    }
}
fn watch_external(
    watcher: &mut impl Watcher,
    root: &Path,
    observed: &BTreeSet<PathBuf>,
    watched: &mut BTreeSet<PathBuf>,
) -> Result<(), Error> {
    for input in observed.iter().filter(|path| !path.starts_with(root)) {
        let mut directory = if input.is_dir() {
            input.as_path()
        } else {
            input.parent().unwrap()
        };
        while !directory.is_dir() {
            directory = directory.parent().unwrap();
        }
        if !watched.contains(directory) {
            watcher
                .watch(directory, RecursiveMode::NonRecursive)
                .map_err(|error| Error::Message(error.to_string()))?;
            watched.insert(directory.into());
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
    if path == Path::new("_notist/preview/events") && request.method() == &Method::Get {
        events(request, state);
        return;
    }
    if path == Path::new("_notist/preview/client.js") {
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
            "<!doctype html><title>Notist preview</title><p>Waiting for a successful build…</p><script type=\"module\" src=\"{}\"></script>",
            preview_url(
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
        let url = format!("/{}/", notist_ssg::file_url(&file));
        let _ = request.respond(Response::empty(308).with_header(header("Location", &url)));
        return;
    }
    if file == Path::new("index.html")
        && !site.files.contains_key(&file)
        && let Some(first) = site.pages.first()
    {
        let _ =
            request.respond(Response::empty(302).with_header(header("Location", &first.route.url)));
        return;
    }
    let Some(bytes) = site.files.get(&file) else {
        drop(published);
        simple(
            request,
            404,
            b"Page not found".to_vec(),
            "text/plain; charset=utf-8",
        );
        return;
    };
    let mut bytes = bytes.clone();
    let revision = published.revision;
    drop(published);
    if file
        .extension()
        .is_some_and(|extension| extension == "html")
    {
        let html = String::from_utf8_lossy(&bytes);
        let script = format!(
            "<script type=\"module\" src=\"{}\"></script>",
            preview_url(&file, revision)
        );
        bytes = if let Some(offset) = html.rfind("</body>") {
            format!("{}{}{}", &html[..offset], script, &html[offset..]).into_bytes()
        } else {
            format!("{html}{script}").into_bytes()
        };
    }
    simple(request, 200, bytes, mime(&file));
}
fn preview_url(file: &Path, revision: u64) -> String {
    format!(
        "{}{}_notist/preview/client.js?v={revision}",
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
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
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
        let data = serde_json::json!({"revision":published.revision,"error":published.error});
        drop(published);
        let message = if previous != Some(event) {
            previous = Some(event);
            Some(format!("event: update\ndata: {data}\n\n"))
        } else if heartbeat.elapsed() > Duration::from_secs(10) {
            Some(": heartbeat\n\n".into())
        } else {
            None
        };
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
