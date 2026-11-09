use notist::resources::{ResourceError, ResourceKind, ResourceReader};
use notist::{
    EvaluationInputs, FsResources, InputError, InputLimits, InputRead, MemoryResources,
    OverlayResources, ResourcePath, ResourceSession, Resources, Vault,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

fn path(root: &str, name: &str) -> ResourcePath {
    ResourcePath::resolve(root, "doc.not", name).unwrap()
}
fn ready(read: InputRead) -> Arc<[u8]> {
    let InputRead::Ready(bytes) = read else {
        panic!("{read:?}")
    };
    bytes
}
fn failed(read: InputRead, expected: InputError) {
    assert!(
        matches!(read, InputRead::Failed(ref error) if *error == expected),
        "{read:?} != {expected:?}"
    );
}

struct Observed {
    files: MemoryResources,
    reads: Arc<Mutex<BTreeMap<PathBuf, usize>>>,
    limits: Arc<Mutex<Vec<usize>>>,
    kinds: Arc<AtomicUsize>,
    resolutions: Arc<AtomicUsize>,
    responses: Arc<Mutex<BTreeMap<PathBuf, InputRead>>>,
}
impl Observed {
    fn new() -> Self {
        Self {
            files: MemoryResources::new("/project"),
            reads: Arc::default(),
            limits: Arc::default(),
            kinds: Arc::default(),
            resolutions: Arc::default(),
            responses: Arc::default(),
        }
    }
}
impl Resources for Observed {
    fn root(&self) -> &Path {
        self.files.root()
    }
    fn resolve(&self, path: &Path) -> PathBuf {
        self.resolutions.fetch_add(1, Ordering::Relaxed);
        self.files.resolve(path)
    }
    fn read(&self, _: &Path) -> Result<Vec<u8>, ResourceError> {
        panic!("bounded hosts must not fall back to unbounded reads")
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        self.kinds.fetch_add(1, Ordering::Relaxed);
        self.files.kind(path)
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        self.files.entries(path)
    }
    fn read_root(&self, root: &Path) -> Result<Arc<dyn ResourceReader>, InputError> {
        Ok(Arc::new(Reader {
            root: self.resolve(root),
            base: self.files.read_root(root)?,
            reads: self.reads.clone(),
            limits: self.limits.clone(),
            responses: self.responses.clone(),
        }))
    }
}
struct Reader {
    root: PathBuf,
    base: Arc<dyn ResourceReader>,
    reads: Arc<Mutex<BTreeMap<PathBuf, usize>>>,
    limits: Arc<Mutex<Vec<usize>>>,
    responses: Arc<Mutex<BTreeMap<PathBuf, InputRead>>>,
}
impl ResourceReader for Reader {
    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead {
        *self
            .reads
            .lock()
            .unwrap()
            .entry(self.root.join(path))
            .or_default() += 1;
        self.limits.lock().unwrap().push(limit);
        self.responses
            .lock()
            .unwrap()
            .get(&self.root.join(path))
            .cloned()
            .unwrap_or_else(|| self.base.read_bounded(path, limit))
    }
}

#[test]
fn cached_source_reads_resolve_once_including_errors_and_aliases() {
    let mut resources = Observed::new();
    resources.files.insert("source.notc", "shared source");
    resources.files.insert("bad.notc", vec![0xff]);
    let session = ResourceSession::new(&resources).with_read_root("/project");
    let source = session.source_shared(Path::new("source.notc")).unwrap();
    assert!(session.source_shared(Path::new("bad.notc")).is_err());
    assert!(session.source_shared(Path::new("missing.notc")).is_err());

    resources.resolutions.store(0, Ordering::Relaxed);
    let alias = session
        .source_shared(Path::new("nested/../source.notc"))
        .unwrap();
    assert!(Arc::ptr_eq(&source, &alias));
    assert_eq!(resources.resolutions.load(Ordering::Relaxed), 1);
    assert_eq!(
        session.read(Path::new("./source.notc")).unwrap(),
        b"shared source"
    );
    assert_eq!(resources.resolutions.load(Ordering::Relaxed), 2);
    assert!(matches!(
        session.source_shared(Path::new("./bad.notc")),
        Err(ResourceError::InvalidUtf8(path)) if path == Path::new("/project/bad.notc")
    ));
    assert_eq!(resources.resolutions.load(Ordering::Relaxed), 3);
    for shared in [false, true] {
        let error = if shared {
            session
                .source_shared(Path::new("./missing.notc"))
                .unwrap_err()
        } else {
            session.read(Path::new("./missing.notc")).unwrap_err()
        };
        assert!(
            matches!(error, ResourceError::NotFound(path) if path == Path::new("/project/missing.notc"))
        );
    }
    assert_eq!(resources.resolutions.load(Ordering::Relaxed), 5);
}

#[test]
fn roots_are_explicit_and_cached_source_bytes_do_not_grant_data_access() {
    let mut files = MemoryResources::new("/project");
    files.insert("data.txt", "private");
    files.insert("../module/lib.notc", "source");
    let session = ResourceSession::new(files).with_read_root("/project");
    let data = path("/project", "data.txt");
    assert_eq!(ready(session.file(&data)).as_ref(), b"private");
    assert_eq!(
        session.source(Path::new("/module/lib.notc")).unwrap(),
        "source"
    );
    let external = path("/module", "lib.notc");
    failed(session.file(&external), InputError::AccessDenied);
    // Changing Path.root cannot manufacture a grant for an absolute target.
    let forged = path("/", "/module/lib.notc");
    failed(session.file(&forged), InputError::AccessDenied);
    assert_eq!(session.dependencies().len(), 3);
    let denied = ResourceSession::new(MemoryResources::new("/project"));
    failed(denied.file(&data), InputError::AccessDenied);
}

#[test]
fn shared_text_hits_retain_identity_utf8_errors_and_request_limits() {
    let mut resources = Observed::new();
    resources.files.insert("source.notc", "shared source");
    resources.files.insert("bad.notc", vec![0xff]);
    let session = ResourceSession::new(&resources).with_read_root("/project");
    let first = session.source_shared(Path::new("source.notc")).unwrap();
    let second = session.source_shared(Path::new("./source.notc")).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    for _ in 0..2 {
        assert!(matches!(
            session.source_shared(Path::new("bad.notc")),
            Err(ResourceError::InvalidUtf8(_))
        ));
    }
    assert!(
        resources
            .reads
            .lock()
            .unwrap()
            .values()
            .all(|&calls| calls == 1)
    );
    let limited = ResourceSession::new(&resources)
        .with_read_root("/project")
        .with_limits(InputLimits {
            requests: 1,
            ..Default::default()
        });
    assert!(limited.source_shared(Path::new("source.notc")).is_ok());
    assert!(
        limited
            .source_shared(Path::new("source.notc"))
            .unwrap_err()
            .to_string()
            .contains("request limit exceeded")
    );
    let denied = ResourceSession::new(&resources);
    assert!(denied.source_shared(Path::new("source.notc")).is_ok());
    failed(
        denied.file(&path("/project", "source.notc")),
        InputError::AccessDenied,
    );
}

#[test]
fn successes_failures_source_and_data_share_one_loading_cache_across_pages() {
    let mut resources = Observed::new();
    resources.files.insert(
        "lib.notc",
        "fn show(source: Path) -> BlockContent { [ #text(text: read_text(source)) ] }",
    );
    resources.files.insert("data.txt", "shared text");
    let mut vault = Vault::new(&resources);
    for page in ["a.not", "b.not"] {
        let output = vault.render_html(page, "#import \"lib.notc\" as view;\n#view.show(p\"data.txt\")\n\n#view.show(p\"missing.txt\")", Default::default()).unwrap();
        assert!(output.rendered.html.contains("shared text"));
        assert_eq!(output.evaluation.dependencies.len(), 2);
        assert!(
            output
                .evaluation
                .diagnostics
                .iter()
                .any(|d| d.message.contains("resource not found"))
        );
    }
    // The same .notc file as data uses the source version, not a second load.
    let output = vault
        .render_html(
            "c.not",
            "#import \"lib.notc\" as view;\n#view.show(p\"lib.notc\")",
            Default::default(),
        )
        .unwrap();
    assert!(output.rendered.html.contains("fn show"));
    assert!(
        resources
            .reads
            .lock()
            .unwrap()
            .values()
            .all(|count| *count == 1)
    );
    assert_eq!(resources.reads.lock().unwrap().len(), 3);
    assert_eq!(vault.input_session().dependencies().len(), 3);
}

#[test]
fn repeated_reads_do_no_backend_work_and_each_new_file_is_loaded_once() {
    let mut resources = Observed::new();
    for i in 0..256 {
        resources
            .files
            .insert(format!("{i}.txt"), format!("input-{i}"));
    }
    let session = ResourceSession::new(&resources).with_read_root("/project");
    for _ in 0..3 {
        for i in 0..256 {
            let name = format!("{i}.txt");
            if i % 2 == 0 {
                assert_eq!(
                    ready(session.file(&path("/project", &name))).as_ref(),
                    format!("input-{i}").as_bytes()
                );
            }
            let source = session.source(Path::new(&name)).unwrap();
            assert_eq!(
                ready(session.file(&path("/project", &name))).as_ref(),
                source.as_bytes()
            );
        }
    }
    assert_eq!(resources.reads.lock().unwrap().len(), 256);
    assert!(
        resources
            .reads
            .lock()
            .unwrap()
            .values()
            .all(|&calls| calls == 1)
    );
    assert_eq!(resources.kinds.load(Ordering::Relaxed), 0);
    assert_eq!(
        session.cached_bytes(),
        (0..256).map(|i| format!("input-{i}").len()).sum::<usize>()
    );
}

#[test]
fn pending_and_io_failures_are_fixed_without_poisoning_unrelated_inputs() {
    for initial in [
        InputRead::Pending,
        InputRead::Failed(InputError::Io("temporarily unavailable".into())),
    ] {
        let mut resources = Observed::new();
        resources.files.insert("other", "independent");
        let absolute = PathBuf::from("/project/input");
        resources
            .responses
            .lock()
            .unwrap()
            .insert(absolute.clone(), initial.clone());
        let session = ResourceSession::new(&resources).with_read_root("/project");
        let input = path("/project", "input");
        assert_eq!(session.file(&input), initial);
        resources.responses.lock().unwrap().insert(
            absolute.clone(),
            InputRead::Ready(Arc::from(&b"prepared"[..])),
        );
        assert_eq!(session.file(&input), initial);
        assert_eq!(
            ready(session.file(&path("/project", "other"))).as_ref(),
            b"independent"
        );
        assert_eq!(resources.reads.lock().unwrap()[&absolute], 1);
        let fresh = ResourceSession::new(&resources).with_read_root("/project");
        assert_eq!(ready(fresh.file(&input)).as_ref(), b"prepared");
        assert_eq!(resources.reads.lock().unwrap()[&absolute], 2);
    }
}

#[test]
fn data_parse_cache_hits_still_authorize_observe_and_account_for_input_requests() {
    let mut resources = Observed::new();
    resources.files.insert("view.notc",r#"fn show(source:Path=p"data.json")->InlineContent{let name=match read_json(source){ok(d)=>data_expect<String>(data_require<String>(d,"name")),err(e)=>data_error_kind(e)};[#text(text:name)]}"#);
    resources.files.insert("data.json", r#"{"name":"loaded"}"#);
    let mut vault = Vault::new(&resources);
    for page in ["one.not", "two.not"] {
        let out = vault
            .render_html(
                page,
                "#import \"view.notc\" as view;\n#view.show()",
                Default::default(),
            )
            .unwrap();
        assert!(out.evaluation.diagnostics.is_empty());
        assert!(out.rendered.html.contains("loaded"));
        assert_eq!(out.evaluation.dependencies.len(), 1);
    }
    assert_eq!(
        resources.reads.lock().unwrap()[Path::new("/project/data.json")],
        1
    );
    assert_eq!(vault.input_session().data_cache_stats().parses, 1);
    let mut vault = vault.with_read_roots(vec![]);
    let out = vault
        .render_html(
            "three.not",
            "#import \"view.notc\" as view;\n#view.show()",
            Default::default(),
        )
        .unwrap();
    assert!(out.rendered.html.contains("access_denied"));
    assert!(!out.rendered.html.contains("loaded"));
    assert_eq!(vault.input_session().data_cache_stats().parses, 1);
}

#[test]
fn parsed_data_versions_are_not_replaced_and_diagnostics_keep_the_consumed_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("view.notc"),"fn show(source:Path=p\"data.json\")->InlineContent{let d=data_expect<Data>(read_json(source));let value=data_expect<String>(data_require<String>(d,\"name\"));[#text(text:value)]}").unwrap();
    let data = r#"{"name":false}"#;
    std::fs::write(directory.path().join("data.json"), data).unwrap();
    let mut vault = Vault::new(FsResources::new(directory.path()));
    let page = "#import \"view.notc\" as view;\n#view.show()";
    let out = vault
        .render_html("one.not", page, Default::default())
        .unwrap();
    let name = out.evaluation.diagnostics[0]
        .source
        .as_ref()
        .unwrap()
        .clone();
    std::fs::write(
        directory.path().join("data.json"),
        r#"{"name":"new data version"}"#,
    )
    .unwrap();
    assert_eq!(
        out.evaluation.sources.text(name.id()).unwrap().as_ref(),
        data
    );
    let cached = vault
        .render_html("two.not", page, Default::default())
        .unwrap();
    assert_eq!(
        cached.evaluation.sources.text(name.id()).unwrap().as_ref(),
        data
    );
    assert_eq!(cached.evaluation.diagnostics.len(), 1);
    assert_eq!(vault.input_session().data_cache_stats().parses, 1);
    let fresh = Vault::new(FsResources::new(directory.path()))
        .render_html("three.not", page, Default::default())
        .unwrap();
    assert!(fresh.evaluation.diagnostics.is_empty());
    assert!(fresh.rendered.html.contains("new data version"));
    assert_eq!(
        out.evaluation.sources.text(name.id()).unwrap().as_ref(),
        data
    );
}

#[test]
fn memory_and_overlay_reads_check_known_lengths_before_copying() {
    let mut files = MemoryResources::new("/project");
    files.insert("large", vec![b'x'; 4096]);
    let overrides = BTreeMap::from([(PathBuf::from("overlay"), "x".repeat(4096))]);
    let overlay = OverlayResources::new(&files, &overrides);
    let session = ResourceSession::new(overlay)
        .with_read_root("/project")
        .with_limits(InputLimits {
            file_bytes: 32,
            ..Default::default()
        });
    for name in ["large", "overlay"] {
        failed(session.file(&path("/project", name)), InputError::TooLarge);
    }
    assert_eq!(session.cached_bytes(), 0);
}

#[test]
fn cache_and_request_budgets_apply_on_hits_and_bound_backend_allocation() {
    let mut resources = Observed::new();
    resources.files.insert("a", "123456");
    resources.files.insert("b", "123456");
    let session = ResourceSession::new(&resources)
        .with_read_root("/project")
        .with_limits(InputLimits {
            file_bytes: 8,
            cached_bytes: 8,
            requests: 3,
        });
    let a = path("/project", "a");
    let b = path("/project", "b");
    let first = ready(session.file(&a));
    let second = ready(session.file(&a));
    assert!(Arc::ptr_eq(&first, &second));
    failed(session.file(&b), InputError::CacheLimit);
    failed(session.file(&a), InputError::RequestLimit);
    assert_eq!(*resources.limits.lock().unwrap(), [8, 2]);
    assert_eq!(session.cached_bytes(), 6);
}

#[test]
fn filesystem_changes_and_newly_created_missing_files_require_a_new_session() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("source.notc"), "old source").unwrap();
    let session =
        ResourceSession::new(FsResources::new(directory.path())).with_read_root(directory.path());
    assert_eq!(
        session.source(Path::new("source.notc")).unwrap(),
        "old source"
    );
    std::fs::write(
        directory.path().join("source.notc"),
        "new source with different size",
    )
    .unwrap();
    assert_eq!(
        ready(session.file(&path(directory.path().to_str().unwrap(), "source.notc"))).as_ref(),
        b"old source",
    );
    assert_eq!(
        session.source(Path::new("source.notc")).unwrap(),
        "old source"
    );
    let next =
        ResourceSession::new(FsResources::new(directory.path())).with_read_root(directory.path());
    assert_eq!(
        ready(next.file(&path(directory.path().to_str().unwrap(), "source.notc"))).as_ref(),
        b"new source with different size"
    );
    let missing = path(directory.path().to_str().unwrap(), "missing");
    failed(next.file(&missing), InputError::NotFound);
    std::fs::write(directory.path().join("missing"), "created").unwrap();
    failed(next.file(&missing), InputError::NotFound);
    let fresh =
        ResourceSession::new(FsResources::new(directory.path())).with_read_root(directory.path());
    assert_eq!(ready(fresh.file(&missing)).as_ref(), b"created");
}

#[test]
fn a_batch_is_lazy_not_an_atomic_directory_snapshot() {
    let directory = tempfile::tempdir().unwrap();
    for name in ["first", "later"] {
        std::fs::write(directory.path().join(name), "before").unwrap();
    }
    let session =
        ResourceSession::new(FsResources::new(directory.path())).with_read_root(directory.path());
    let p = |name| path(directory.path().to_str().unwrap(), name);
    let first = ready(session.file(&p("first")));
    std::fs::write(directory.path().join("first"), "after").unwrap();
    std::fs::write(directory.path().join("later"), "after").unwrap();
    assert!(Arc::ptr_eq(&first, &ready(session.file(&p("first")))));
    assert_eq!(ready(session.file(&p("later"))).as_ref(), b"after");
    std::fs::remove_file(directory.path().join("later")).unwrap();
    assert_eq!(ready(session.file(&p("later"))).as_ref(), b"after");
    let fresh =
        ResourceSession::new(FsResources::new(directory.path())).with_read_root(directory.path());
    assert_eq!(ready(fresh.file(&p("first"))).as_ref(), b"after");
    failed(fresh.file(&p("later")), InputError::NotFound);
}

#[test]
fn configuration_discovery_keeps_its_first_result_until_the_next_batch() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = Vault::open(directory.path());
    let analysis = vault.analyze("virtual.not", "visible text").unwrap();
    std::fs::write(directory.path().join("Notist.toml"), "").unwrap();
    let evaluated = vault.evaluate("virtual.not", &analysis).unwrap();
    assert!(evaluated.diagnostics.is_empty());
    assert!(
        vault
            .environment_for("other.not", "other text")
            .unwrap()
            .config_path()
            .is_none()
    );
    let mut fresh = Vault::open(directory.path());
    assert_eq!(
        fresh
            .environment_for("virtual.not", "visible text")
            .unwrap()
            .config_path(),
        Some(directory.path().join("Notist.toml").as_path())
    );
}

#[test]
fn physical_roots_enforce_symlink_boundaries_and_regular_bounded_files() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), "secret").unwrap();
    std::fs::create_dir(directory.path().join("folder")).unwrap();
    std::fs::write(directory.path().join("large"), [0; 128]).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(outside.path(), directory.path().join("escape")).unwrap();
        std::os::unix::fs::symlink("large", directory.path().join("inside")).unwrap();
    }
    let session = ResourceSession::new(FsResources::new(directory.path()))
        .with_read_root(directory.path())
        .with_limits(InputLimits {
            file_bytes: 16,
            ..Default::default()
        });
    let p = |name| path(directory.path().to_str().unwrap(), name);
    failed(session.file(&p("folder")), InputError::NotFile);
    failed(session.file(&p(".")), InputError::NotFile);
    failed(session.file(&p("large")), InputError::TooLarge);
    #[cfg(unix)]
    {
        failed(session.file(&p("escape/secret")), InputError::AccessDenied);
        // The filesystem policy allows only symlinks contained by its Dir cap.
        failed(session.file(&p("inside")), InputError::TooLarge);
    }
}

#[cfg(unix)]
#[test]
fn source_or_data_cache_hits_cannot_bypass_a_narrower_root_capability() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("public")).unwrap();
    std::fs::write(directory.path().join("private.txt"), "private").unwrap();
    std::os::unix::fs::symlink("../private.txt", directory.path().join("public/alias.txt"))
        .unwrap();
    let target = path(directory.path().to_str().unwrap(), "public/alias.txt");
    for data_first in [false, true] {
        let session = ResourceSession::new(FsResources::new(directory.path()))
            .with_read_root(directory.path());
        if data_first {
            assert_eq!(ready(session.file(&target)).as_ref(), b"private");
        } else {
            assert_eq!(
                session.source(Path::new("public/alias.txt")).unwrap(),
                "private"
            );
        }
        // The same logical path is now rooted more narrowly. Its old bytes
        // were never authorized by this capability, even though they are cached.
        let session = session.with_read_root(directory.path().join("public"));
        failed(session.file(&target), InputError::AccessDenied);
        failed(session.file(&target), InputError::AccessDenied);
        let session = session.with_read_roots(vec![]);
        failed(session.file(&target), InputError::AccessDenied);
    }
}

#[test]
fn legacy_source_bytes_do_not_manufacture_a_bounded_data_capability() {
    struct SourceOnly(MemoryResources);
    impl Resources for SourceOnly {
        fn root(&self) -> &Path {
            self.0.root()
        }
        fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
            self.0.read(path)
        }
        fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
            self.0.kind(path)
        }
        fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
            self.0.entries(path)
        }
    }
    let mut files = MemoryResources::new("/project");
    files.insert("text", "source-only");
    let session = ResourceSession::new(SourceOnly(files)).with_read_root("/project");
    assert_eq!(session.source(Path::new("text")).unwrap(), "source-only");
    failed(
        session.file(&path("/project", "text")),
        InputError::Unavailable,
    );
}

#[test]
fn logical_source_overrides_and_virtual_documents_are_visible_to_read_text() {
    let mut files = MemoryResources::new("/project");
    files.insert(
        "lib.notc",
        "fn show(source: Path) -> BlockContent { [ #text(text: read_text(source)) ] }",
    );
    files.insert("doc.not", "stale disk text");
    let overrides = BTreeMap::from([(PathBuf::from("data.txt"), "unsaved input".into())]);
    let mut vault = Vault::new(OverlayResources::new(&files, &overrides));
    for page in ["doc.not", "unlanded.not"] {
        let source = "#import \"lib.notc\" as view;\n#view.show(p\"data.txt\")\n\n#view.show(p\"./unlanded.not\")";
        let source = if page == "doc.not" {
            source.replace("unlanded.not", "doc.not")
        } else {
            source.into()
        };
        let output = vault
            .render_html(page, &source, Default::default())
            .unwrap();
        assert!(
            output.evaluation.diagnostics.is_empty(),
            "{:?}",
            output.evaluation.diagnostics
        );
        assert!(output.rendered.html.contains("unsaved input"));
        assert!(output.rendered.html.contains("#import"));
        assert!(!output.rendered.html.contains("stale disk text"));
    }
    let virtual_source = "virtual input text";
    vault.analyze("generated.not", virtual_source).unwrap();
    let output = vault
        .render_html(
            "reader.not",
            "#import \"lib.notc\" as view;\n#view.show(p\"generated.not\")",
            Default::default(),
        )
        .unwrap();
    assert!(output.evaluation.diagnostics.is_empty());
    assert!(output.rendered.html.contains(virtual_source));
}

#[test]
fn later_document_inputs_do_not_replace_cached_data_or_poison_the_batch() {
    for existing in [false, true] {
        let mut files = MemoryResources::new("/project");
        files.insert(
            "view.notc",
            "fn show(source: Path) -> BlockContent { [ #text(text: read_text(source)) ] }",
        );
        if existing {
            files.insert("target.not", "old input");
        }
        let mut vault = Vault::new(files);
        let source = "#import \"view.notc\" as view;\n#view.show(p\"target.not\")";
        let analysis = vault.analyze("reader.not", source).unwrap();
        let before = vault
            .render_output("reader.not", &analysis, Default::default())
            .unwrap();
        vault.analyze("target.not", "new virtual text").unwrap();
        let after = vault
            .render_output("reader.not", &analysis, Default::default())
            .unwrap();
        assert_eq!(before.rendered.html, after.rendered.html);
        if existing {
            assert!(after.rendered.html.contains("old input"));
            assert!(after.evaluation.diagnostics.is_empty());
        } else {
            assert!(
                after
                    .evaluation
                    .diagnostics
                    .iter()
                    .any(|d| d.message.contains("resource not found"))
            );
        }
        let mut fresh = Vault::new(vault.resources());
        fresh.analyze("target.not", "new virtual text").unwrap();
        let next = fresh
            .render_html("reader.not", source, Default::default())
            .unwrap();
        assert!(next.evaluation.diagnostics.is_empty());
        assert!(next.rendered.html.contains("new virtual text"));
    }
}

#[test]
fn document_input_registration_keeps_the_first_text_until_a_new_batch() {
    let mut files = MemoryResources::new("/project");
    files.insert(
        "view.notc",
        "fn show(source: Path) -> BlockContent { [ #text(text: read_text(source)) ] }",
    );
    let first = "#import \"view.notc\" as view;\n#view.show(p\"doc.not\")\n\nfirst";
    let second = first.replace("first", "second");
    let mut vault = Vault::new(&files);
    let a = vault.analyze("doc.not", first).unwrap();
    let b = vault.analyze("doc.not", &second).unwrap();
    for analysis in [&b, &a] {
        let evaluated = vault.evaluate("doc.not", analysis).unwrap();
        assert!(evaluated.diagnostics.is_empty());
        assert!(
            evaluated
                .root
                .descendants()
                .any(|n| n.field("text") == Some(&notist::Value::Str(first.into())))
        );
    }
    let mut fresh = Vault::new(&files);
    let b = fresh.analyze("doc.not", &second).unwrap();
    let evaluated = fresh.evaluate("doc.not", &b).unwrap();
    assert!(evaluated.diagnostics.is_empty());
    assert!(
        evaluated
            .root
            .descendants()
            .any(|n| n.field("text") == Some(&notist::Value::Str(second.clone().into())))
    );
}

#[test]
fn importing_external_modules_does_not_grant_their_adjacent_data() {
    let mut files = MemoryResources::new("/project");
    files.insert(
        "../external/lib.notc",
        "fn show() -> BlockContent { [ #text(text: read_text(p\"./private.txt\")) ] }",
    );
    files.insert("../external/private.txt", "private text");
    let source = "#import \"../external/lib.notc\" as view;\n#view.show()";
    let output = Vault::new(&files)
        .render_html("doc.not", source, Default::default())
        .unwrap();
    assert!(
        output.analysis.diagnostics().is_empty(),
        "{:?}",
        output.analysis.diagnostics()
    );
    assert!(
        output
            .evaluation
            .diagnostics
            .iter()
            .any(|d| d.message.contains("access denied"))
    );
    let output = Vault::new(files)
        .with_read_roots(["/project".into(), "/external".into()])
        .render_html("doc.not", source, Default::default())
        .unwrap();
    assert!(output.evaluation.diagnostics.is_empty());
    assert!(output.rendered.html.contains("private text"));
}

#[test]
fn aliases_reexports_and_comptime_arguments_cannot_hide_markup_reads() {
    let mut files = MemoryResources::new("/project");
    files.insert("leaf.notc", "fn load(source: Path) -> String { read_text(source) } fn fixed<message: String>() -> BlockContent { [ #text(text: message) ] }");
    files.insert(
        "exports.notc",
        "import \"leaf.notc\": load as fetch, fixed;",
    );
    let mut vault = Vault::new(files);
    for tail in [
        "#fetch(p\"missing.txt\")",
        "#text(text: fetch(p\"missing.txt\"))",
        "#fixed<fetch(p\"missing.txt\")>()",
    ] {
        let source = format!("#import \"exports.notc\": *;\n{tail}");
        let analysis = vault.analyze("page.not", &source).unwrap();
        assert!(!analysis.diagnostics().is_empty());
        let _ = vault.evaluate("page.not", &analysis).unwrap();
        assert!(vault.input_session().dependencies().is_empty());
    }
}
