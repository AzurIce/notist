use notist::resources::ResourceKind;
use notist::{
    MemoryResources, OverlayResources, PreparedInputs, RenderOptions, ResourceError, Resources,
    Vault, VaultError,
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

fn memory() -> MemoryResources {
    let mut resources = MemoryResources::new("vault");
    resources.insert(
        "Notist.toml",
        b"[dependencies]\nwidgets = {path = 'widgets'}".to_vec(),
    );
    resources.insert(
        "widgets/Notist.toml",
        b"[package]\nname = 'widgets'".to_vec(),
    );
    resources.insert("widgets/lib.notc", b"fn panel(title: String = \"outer\")[children: Content] -> Content; fn badge(label: String) -> InlineContent;".to_vec());
    resources.insert("widgets/components/panel/index.js", Vec::new());
    resources.insert("widgets/components/badge.js", Vec::new());
    resources
}
const DOCUMENT: &str = "#widgets::panel()[#widgets::badge(\"inner\")]";

#[test]
fn typed_output_preserves_nested_components_maps_and_published_urls() {
    let urls = BTreeMap::from([
        (
            PathBuf::from("widgets/components/panel/index.js"),
            "https://host.test/panel/index.js".into(),
        ),
        (
            PathBuf::from("widgets/components/badge.js"),
            "https://host.test/badge.js".into(),
        ),
    ]);
    let mut vault = Vault::new(memory()).with_module_urls(urls);
    let output = vault
        .render_html("notes/doc.not", DOCUMENT, RenderOptions::default())
        .unwrap();
    assert!(output.analysis.diagnostics().is_empty());
    assert!(output.rendered.diagnostics.is_empty());
    assert_eq!(output.path, Path::new("/vault/notes/doc.not"));
    assert_eq!(output.rendered.used_components.len(), 2);
    assert_eq!(
        output.rendered.used_components[0].module.url(),
        Some("https://host.test/panel/index.js")
    );
    assert!(
        output
            .rendered
            .source_map
            .iter()
            .any(|mapping| mapping.kind == notist_html::SourceMappingKind::Container)
    );
    assert!(output.rendered.source_map.iter().all(|mapping| {
        output
            .rendered
            .html
            .contains(&format!("data-notist-node=\"{}\"", mapping.node_id))
            && mapping.range.end <= DOCUMENT.len()
    }));
    let plain = vault
        .render_html(
            "notes/doc.not",
            DOCUMENT,
            RenderOptions { source_map: false },
        )
        .unwrap();
    assert!(plain.rendered.source_map.is_empty());
    assert!(!plain.rendered.html.contains("data-notist-node"));
}

#[test]
fn nearest_configs_and_document_index_share_environment_selection() {
    let mut resources = memory();
    resources.insert(
        "nested/Notist.toml",
        b"[dependencies]\nwidgets = {path = 'other'}".to_vec(),
    );
    resources.insert(
        "nested/other/Notist.toml",
        b"[package]\nname = 'widgets'".to_vec(),
    );
    resources.insert(
        "nested/other/lib.notc",
        b"fn badge(label: Int) -> InlineContent;".to_vec(),
    );
    resources.insert(
        "outer.not",
        b"#widgets::badge(\"text\") [[nested/inner.not]]".to_vec(),
    );
    resources.insert(
        "nested/inner.not",
        b"#widgets::badge(42) [[../outer.not]]".to_vec(),
    );
    let mut vault = Vault::new(resources);
    assert!(
        vault
            .analyze_resource("outer.not")
            .unwrap()
            .diagnostics()
            .is_empty()
    );
    assert!(
        vault
            .analyze_resource("nested/inner.not")
            .unwrap()
            .diagnostics()
            .is_empty()
    );
    assert!(
        !vault
            .analyze("nested/wrong.not", "#widgets::badge(\"text\")")
            .unwrap()
            .diagnostics()
            .is_empty()
    );
    assert!(vault.index(".").unwrap().check().is_empty());
    // Returning to the outer environment must not keep the nested signature.
    assert!(
        vault
            .analyze("outer.not", "#widgets::badge(\"text\")")
            .unwrap()
            .diagnostics()
            .is_empty()
    );
}

#[test]
fn overlays_include_unsaved_configs_and_declarations_and_errors_keep_source() {
    let base = memory();
    let overlays = BTreeMap::from([
        (
            PathBuf::from("unsaved/Notist.toml"),
            "[dependencies]\nwidgets = {path = 'new'}".into(),
        ),
        (
            PathBuf::from("unsaved/new/Notist.toml"),
            "[package]\nname = 'widgets'".into(),
        ),
        (
            PathBuf::from("unsaved/new/lib.notc"),
            "fn badge(label: Int) -> InlineContent;".into(),
        ),
    ]);
    let mut vault = Vault::new(OverlayResources::new(&base, &overlays));
    assert!(
        vault
            .analyze("unsaved/doc.not", "#widgets::badge(7)")
            .unwrap()
            .diagnostics()
            .is_empty()
    );
    let broken_source = "fn badge(label: Int = false) -> InlineContent;";
    let mut broken = overlays;
    broken.insert("unsaved/new/lib.notc".into(), broken_source.into());
    let mut vault = Vault::new(OverlayResources::new(&base, &broken));
    let VaultError::Environment(errors) = vault.analyze("unsaved/doc.not", "Text").unwrap_err()
    else {
        panic!("expected declaration failure")
    };
    assert!(errors.iter().all(
        |error| error.path == Path::new("/vault/unsaved/new/lib.notc")
            && error.source == broken_source
            && !error.diagnostic.span.is_empty()
    ));
    // A new input view assembles afresh, without retaining the failed registry.
    let mut vault = Vault::new(base);
    assert!(
        vault
            .analyze("doc.not", DOCUMENT)
            .unwrap()
            .diagnostics()
            .is_empty()
    );
}

#[test]
fn html_entry_conflicts_do_not_invalidate_semantic_declarations() {
    let mut resources = memory();
    resources.insert("widgets/components/panel.js", Vec::new());
    let mut vault = Vault::new(resources);
    assert!(
        vault
            .analyze("doc.not", DOCUMENT)
            .unwrap()
            .diagnostics()
            .is_empty()
    );
    let VaultError::Environment(errors) = vault
        .render_html("doc.not", DOCUMENT, RenderOptions::default())
        .unwrap_err()
    else {
        panic!("expected target conflict")
    };
    assert!(errors[0].diagnostic.message.contains("conflicting"));
    assert_eq!(errors[0].path, Path::new("/vault/widgets/lib.notc"));
    assert!(errors[0].source.starts_with("fn panel"));

    let mut resources = MemoryResources::new("vault");
    resources.insert(
        "Notist.toml",
        b"[dependencies]\nwidgets = {path = 'widgets'}".to_vec(),
    );
    resources.insert(
        "widgets/Notist.toml",
        b"[package]\nname = 'widgets'".to_vec(),
    );
    resources.insert(
        "widgets/lib.notc",
        b"fn panel()[children: Content] -> Content;".to_vec(),
    );
    let output = Vault::new(resources)
        .render_html(
            "doc.not",
            "#widgets::panel()[visible]",
            RenderOptions::default(),
        )
        .unwrap();
    assert!(output.analysis.diagnostics().is_empty());
    assert!(output.rendered.html.contains("visible"));
    assert!(
        output.rendered.diagnostics[0]
            .message
            .contains("no HTML implementation")
    );
}

struct Denied(MemoryResources);
impl Resources for Denied {
    fn root(&self) -> &Path {
        self.0.root()
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        self.0.read(path)
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        self.0.entries(path)
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        if path.ends_with("components/badge.js") {
            Err(ResourceError::Access {
                path: path.into(),
                message: "host denied resource metadata".into(),
            })
        } else {
            self.0.kind(path)
        }
    }
}
#[test]
fn host_failures_and_invalid_utf8_are_not_treated_as_missing_resources() {
    let mut vault = Vault::new(Denied(memory()));
    assert!(
        vault
            .analyze("doc.not", DOCUMENT)
            .unwrap()
            .diagnostics()
            .is_empty()
    );
    let VaultError::Environment(errors) = vault.html_registry("doc.not").unwrap_err() else {
        panic!("expected access failure")
    };
    assert!(errors[0].diagnostic.message.contains("host denied"));
    let mut resources = MemoryResources::new("vault");
    resources.insert("broken.not", vec![255]);
    let mut vault = Vault::new(&resources);
    assert!(matches!(
        vault.analyze_resource("broken.not"),
        Err(VaultError::Resource(ResourceError::InvalidUtf8(_)))
    ));
    assert!(matches!(
        vault.analyze_resource("missing.not"),
        Err(VaultError::Resource(ResourceError::NotFound(_)))
    ));
}

static LOWER_CALLS: AtomicUsize = AtomicUsize::new(0);
struct CountingFrontend;
impl notist::Frontend for CountingFrontend {
    fn extensions(&self) -> &[&str] {
        &["not"]
    }

    fn compile(&self, source: &str, options: notist::FrontendOptions) -> notist::FrontendOutput {
        LOWER_CALLS.fetch_add(1, Ordering::SeqCst);
        notist::Frontend::compile(&notist::NotistFrontend, source, options)
    }
}

#[test]
fn rendering_and_debugging_each_lower_exactly_once_and_respect_overrides() {
    let pipeline = notist::Pipeline::default().with_frontend(CountingFrontend);
    let mut vault = Vault::new(memory()).with_pipeline(pipeline);
    LOWER_CALLS.store(0, Ordering::SeqCst);
    let output = vault
        .render_html("doc.not", DOCUMENT, RenderOptions::default())
        .unwrap();
    assert_eq!(LOWER_CALLS.load(Ordering::SeqCst), 1);
    let (analysis, inspection) = vault.inspect("doc.not", DOCUMENT).unwrap();
    vault
        .render_output("doc.not", analysis.root(), RenderOptions::default())
        .unwrap();
    assert_eq!(LOWER_CALLS.load(Ordering::SeqCst), 2);
    assert!(
        inspection
            .syntax
            .as_deref()
            .unwrap()
            .as_any()
            .downcast_ref::<notist_syntax::syntax::SyntaxNode>()
            .is_some()
    );
    assert_eq!(analysis, output.analysis);
}

#[test]
fn scans_follow_sibling_documents_within_the_vault_using_their_own_configs() {
    let mut resources = MemoryResources::new("/repo");
    resources.insert(
        "docs/README.not",
        b"@(id: \"root\")\n= Docs\n\n[Example](../packages/demo/README.not#anchor) [Missing](../packages/missing/README.not) [Remote](https://example.test/README.not)".to_vec(),
    );
    resources.insert(
        "packages/demo/Notist.toml",
        b"[package]\nname = 'demo'".to_vec(),
    );
    resources.insert(
        "packages/demo/lib.notc",
        b"fn badge(label: String) -> InlineContent;".to_vec(),
    );
    resources.insert(
        "packages/demo/README.not",
        b"@(id: \"anchor\")\n= Package\n\n#demo::badge(\"hello\")\n\n[Back](../../docs/README.not#root) [Bad anchor](../../docs/README.not#missing)".to_vec(),
    );
    resources.insert(
        "packages/unrelated/README.not",
        b"#unregistered::function()".to_vec(),
    );
    let index = Vault::new(resources).index("docs").unwrap();
    let diagnostics = index.check();
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert!(diagnostics.iter().any(|(path, diagnostic)| {
        path == Path::new("docs/README.not")
            && diagnostic.message.contains("packages/missing/README.not")
    }));
    assert!(diagnostics.iter().any(|(path, diagnostic)| {
        path == Path::new("packages/demo/README.not")
            && diagnostic.message.contains("missing item `#missing`")
    }));
    assert_eq!(index.backlinks(Path::new("docs/README.not")).len(), 2);
    assert_eq!(
        index.backlinks(Path::new("packages/demo/README.not"))[0].0,
        Path::new("docs/README.not")
    );
}

struct NoOutsideReads(MemoryResources);
impl Resources for NoOutsideReads {
    fn root(&self) -> &Path {
        self.0.root()
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        assert!(
            self.resolve(path).starts_with(self.root()),
            "outside read: {path:?}"
        );
        self.0.read(path)
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        // Nearest configuration discovery may inspect parent directories.
        assert!(
            !self.resolve(path).starts_with(Path::new("/repo/outside")),
            "outside metadata: {path:?}"
        );
        self.0.kind(path)
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        self.0.entries(path)
    }
}

#[test]
fn local_links_and_embeds_outside_the_vault_are_diagnosed_without_loading() {
    let source = "[outside](../../outside/broken.not#missing) ![image](../../outside/image.svg) [prefix](/repo/docs-other/page.not) [remote](https://example.test/page.not) [inside](../README.not#home)";
    let mut resources = MemoryResources::new("/repo/docs");
    resources.insert("README.not", b"@(id: \"home\")\n= Home".to_vec());
    resources.insert("../outside/broken.not", vec![255]);
    resources.insert("../outside/Notist.toml", b"invalid = [".to_vec());
    for extension in ["not", "md"] {
        resources.insert(
            format!("notes/page.{extension}"),
            source.as_bytes().to_vec(),
        );
    }
    let mut vault = Vault::new(NoOutsideReads(resources.clone()));
    assert!(matches!(
        vault.index("../outside"),
        Err(VaultError::Resource(ResourceError::Access { .. }))
    ));
    assert!(
        vault
            .analyze("network.not", "#link(\"//example.test/page.not\")[network]")
            .unwrap()
            .diagnostics()
            .is_empty()
    );
    for extension in ["not", "md"] {
        let path = format!("notes/page.{extension}");
        let analysis = vault.analyze(&path, source).unwrap();
        let (inspected, _) = vault.inspect(&path, source).unwrap();
        let output = vault
            .render_html(&path, source, RenderOptions::default())
            .unwrap();
        assert_eq!(analysis, inspected);
        assert_eq!(analysis, output.analysis);
        assert_eq!(
            analysis.diagnostics().len(),
            3,
            "{:?}",
            analysis.diagnostics()
        );
        for diagnostic in analysis.diagnostics() {
            assert_eq!(diagnostic.phase, notist::Phase::Semantic);
            assert!(
                diagnostic
                    .message
                    .contains("outside Vault root `/repo/docs`")
            );
            let span = usize::from(diagnostic.span.start())..usize::from(diagnostic.span.end());
            let reference = &source[span];
            assert!(
                reference.contains("../../outside/") || reference.contains("/repo/docs-other/")
            );
        }
    }
    let index = vault.index("notes").unwrap();
    assert_eq!(index.check().len(), 6, "{:?}", index.check());
    assert_eq!(index.backlinks(Path::new("README.not")).len(), 2);
    assert!(
        index
            .backlinks(Path::new("../outside/broken.not"))
            .is_empty()
    );
    // The pure graph API enforces the same boundary on Pipeline-only results.
    let analysis = notist::Pipeline::default()
        .analyze("notes/page.not", source, notist::builtins::registry())
        .unwrap();
    let pure =
        notist::VaultIndex::from_documents("/repo/docs", [("notes/page.not".into(), analysis)]);
    let outside: Vec<_> = pure
        .check()
        .into_iter()
        .filter(|(_, diagnostic)| diagnostic.message.contains("outside Vault root"))
        .collect();
    assert_eq!(outside.len(), 3);

    let inputs = PreparedInputs {
        root: resources.root().into(),
        config: None,
        files: resources.files().clone(),
        module_urls: BTreeMap::new(),
    };
    let inputs_json = serde_json::to_string(&inputs).unwrap();
    for render in [
        notist::preview::render_prepared,
        notist::preview::analyze_prepared,
    ] {
        let output: serde_json::Value =
            serde_json::from_str(&render("notes/page.md", source, &inputs_json)).unwrap();
        assert_eq!(output["diagnostics"].as_array().unwrap().len(), 3);
        assert!(
            output["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .all(|diagnostic| diagnostic["origin"] == "analysis"
                    && diagnostic["path"] == "notes/page.md")
        );
    }
}

#[test]
fn package_dependencies_outside_the_vault_remain_available() {
    let mut resources = MemoryResources::new("/repo/docs");
    resources.insert(
        "Notist.toml",
        b"[dependencies]\ndemo = {path = '../packages/demo'}".to_vec(),
    );
    resources.insert(
        "../packages/demo/Notist.toml",
        b"[package]\nname = 'demo'".to_vec(),
    );
    resources.insert(
        "../packages/demo/lib.notc",
        b"fn badge(label: String) -> InlineContent;".to_vec(),
    );
    let mut vault = Vault::new(resources);
    let source = "#demo::badge(\"hello\") [package](../packages/demo/README.not)";
    let analysis = vault.analyze("README.not", source).unwrap();
    assert!(
        vault
            .environment_for("README.not")
            .unwrap()
            .registry()
            .resolve("demo::badge")
            .is_ok()
    );
    assert_eq!(analysis.diagnostics().len(), 1);
    assert!(
        analysis.diagnostics()[0]
            .message
            .contains("outside Vault root")
    );
}

#[test]
fn local_and_worker_inputs_use_the_same_real_package_signatures_and_entries() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf();
    let paths = [
        "packages/widgets/README.not",
        "packages/grammar/README.not",
        "packages/typst/README.not",
    ];
    let mut local = Vault::open(&root);
    let mut files = BTreeMap::new();
    for document in paths {
        let environment = local.environment_for(document).unwrap();
        let config = environment.config_path().unwrap();
        files.insert(config.to_path_buf(), std::fs::read(config).unwrap());
        for package in environment.packages().values() {
            let manifest = package.root.join("Notist.toml");
            files.insert(manifest.clone(), std::fs::read(manifest).unwrap());
            files.insert(
                package.root.join("lib.notc"),
                package.source.as_bytes().to_vec(),
            );
        }
        for definition in environment
            .registry()
            .functions()
            .filter(|definition| definition.id.package != "notist")
        {
            let package = &environment.packages()[&definition.id.package];
            for entry in notist_html::components::component_entries(&definition.id.name) {
                let path = package.root.join(entry);
                if path.is_file() {
                    files.insert(path, Vec::new());
                }
            }
        }
    }
    let inputs = PreparedInputs {
        root: root.clone(),
        config: None,
        files,
        module_urls: BTreeMap::new(),
    };
    // Exercise the transport, independent of filesystem availability in the Worker.
    let json = serde_json::to_string(&inputs).unwrap();
    let mut worker = serde_json::from_str::<PreparedInputs>(&json)
        .unwrap()
        .into_vault();
    for path in paths {
        let source = std::fs::read_to_string(root.join(path)).unwrap();
        let disk = local
            .render_html(path, &source, RenderOptions::default())
            .unwrap();
        let prepared = worker
            .render_html(path, &source, RenderOptions::default())
            .unwrap();
        assert!(disk.analysis.diagnostics().is_empty());
        assert!(disk.transformed.diagnostics.is_empty());
        assert!(disk.rendered.diagnostics.is_empty());
        assert_eq!(disk.analysis, prepared.analysis);
        assert_eq!(disk.transformed, prepared.transformed);
        assert_eq!(disk.rendered.html, prepared.rendered.html);
        assert_eq!(disk.rendered.diagnostics, prepared.rendered.diagnostics);
        assert_eq!(disk.rendered.source_map, prepared.rendered.source_map);
        let components = |result: &notist_html::RenderResult| {
            result
                .used_components
                .iter()
                .map(|component| (&component.id, &component.tag, &component.module))
                .map(|(id, tag, module)| (id.clone(), tag.clone(), module.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(components(&disk.rendered), components(&prepared.rendered));
    }
}

#[test]
fn logical_resolution_and_directory_failures_are_consistent_between_hosts() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join("sub")).unwrap();
    std::fs::write(directory.path().join("sub/source.not"), "Text").unwrap();
    let disk = notist::FsResources::new(directory.path());
    let mut memory = MemoryResources::new(directory.path());
    memory.insert("sub/source.not", b"Text".to_vec());
    for resources in [&disk as &dyn Resources, &memory as &dyn Resources] {
        let logical = resources.resolve(Path::new("sub/../sub/source.not"));
        assert_eq!(resources.resolve(&logical), logical);
        assert_eq!(resources.source(&logical).unwrap(), "Text");
        assert!(matches!(
            resources.entries(Path::new("missing")),
            Err(ResourceError::NotFound(_))
        ));
        assert!(matches!(
            resources.entries(Path::new("sub/source.not")),
            Err(ResourceError::Access { .. })
        ));
        assert!(matches!(
            resources.read(Path::new("sub")),
            Err(ResourceError::Access { .. })
        ));
        // The public Vault can borrow a host's resource adapter or trait object.
        assert!(
            Vault::new(resources)
                .analyze_resource("sub/source.not")
                .unwrap()
                .diagnostics()
                .is_empty()
        );
    }
}

#[test]
fn ordinary_browser_and_worker_adapters_preserve_the_complete_render_result() {
    let resources = memory();
    let mut inputs = PreparedInputs {
        root: resources.root().into(),
        config: None,
        files: resources.files().clone(),
        module_urls: BTreeMap::from([
            (
                PathBuf::from("widgets/components/panel/index.js"),
                "https://host.test/panel.js".into(),
            ),
            (
                PathBuf::from("widgets/components/badge.js"),
                "https://host.test/badge.js".into(),
            ),
        ]),
    };
    let message = serde_json::to_string(&inputs).unwrap();
    let plain: serde_json::Value = serde_json::from_str(&notist::preview::render_prepared(
        "doc.not", DOCUMENT, &message,
    ))
    .unwrap();
    let debug: serde_json::Value = serde_json::from_str(&notist::preview::analyze_prepared(
        "doc.not", DOCUMENT, &message,
    ))
    .unwrap();
    assert!(
        plain.get("tree").is_none() && plain.get("ir1").is_none() && plain.get("core").is_none()
    );
    assert!(debug.get("tree").is_some());
    for field in ["html", "source_map", "used_components", "diagnostics"] {
        assert_eq!(plain[field], debug[field]);
    }
    assert!(!plain["source_map"].as_array().unwrap().is_empty());
    let native = inputs
        .clone()
        .into_vault()
        .render_html("doc.not", DOCUMENT, RenderOptions::default())
        .unwrap();
    assert_eq!(plain["html"], native.rendered.html);
    inputs
        .files
        .remove(&PathBuf::from("/vault/widgets/components/panel/index.js"));
    inputs.module_urls.clear();
    let missing: serde_json::Value = serde_json::from_str(&notist::preview::render_prepared(
        "doc.not",
        "#widgets::panel()[visible]",
        &serde_json::to_string(&inputs).unwrap(),
    ))
    .unwrap();
    assert_eq!(missing["diagnostics"][0]["origin"], "render");
    assert_eq!(missing["diagnostics"][0]["path"], "doc.not");
}
