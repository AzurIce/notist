use crate::{
    Error, Page, SiteConfig, assets,
    document::{Document, diagnostic},
    templates,
};
use notist::{
    Diagnostic, DiagnosticCode, Item, Phase, Resources, SourceDiagnostic, Value, Vault,
    publication::{
        Assets,
        url::{fragment, relative_url},
    },
    resources::{ResourceKind, normalize},
};
use serde::Serialize;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
};

/// Owned, validated output. Publishing it is a separate host operation.
#[derive(Debug)]
pub struct BuildOutput {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub pages: Vec<Page>,
    pub warnings: Vec<SourceDiagnostic>,
    pub diagnostics: Vec<SourceDiagnostic>,
    pub data: BTreeMap<String, serde_json::Value>,
}

struct Directory {
    path: String,
    name: String,
    index: Option<usize>,
    pages: Vec<usize>,
    directories: Vec<String>,
}

/// Engine values stay inside the template boundary. Page maps are serialized
/// once; site listings, directory listings and the current page share them.
struct TemplateContext {
    context: tera::Context,
    pages: Vec<tera::Value>,
    directories: BTreeMap<String, tera::Value>,
}

impl TemplateContext {
    fn new(
        config: &SiteConfig,
        pages: &[Page],
        directories: &BTreeMap<String, Directory>,
        data: &BTreeMap<String, serde_json::Value>,
    ) -> Result<Self, Error> {
        use tera::Value;

        let pages: Vec<_> = pages
            .iter()
            .map(Value::try_from_serializable)
            .collect::<Result<_, _>>()
            .map_err(|e| Error::Message(e.to_string()))?;
        let directories: BTreeMap<_, _> = directories
            .iter()
            .map(|(key, directory)| {
                let value = Value::from(BTreeMap::from([
                    ("path", Value::from(directory.path.as_str())),
                    ("name", Value::from(directory.name.as_str())),
                    (
                        "index",
                        directory
                            .index
                            .map_or_else(Value::none, |i| pages[i].clone()),
                    ),
                    (
                        "pages",
                        Value::from(
                            directory
                                .pages
                                .iter()
                                .map(|&i| pages[i].clone())
                                .collect::<Vec<_>>(),
                        ),
                    ),
                    (
                        "directories",
                        Value::from(
                            directory
                                .directories
                                .iter()
                                .map(|key| Value::from(key.as_str()))
                                .collect::<Vec<_>>(),
                        ),
                    ),
                ]));
                (key.clone(), value)
            })
            .collect();
        let site = Value::from(BTreeMap::from([
            ("title", Value::from(config.title.as_str())),
            ("base_url", Value::from(config.base_url.as_str())),
            ("pages", Value::from(pages.clone())),
            ("directories", Value::from(directories.clone())),
            (
                "data",
                Value::try_from_serializable(data).map_err(|e| Error::Message(e.to_string()))?,
            ),
        ]));
        let mut context = tera::Context::new();
        context.insert_value("site", site);
        Ok(Self {
            context,
            pages,
            directories,
        })
    }
}

#[derive(Serialize)]
struct PageAssets<'a> {
    components: Option<&'a String>,
    highlight: String,
    styles: Vec<String>,
    scripts: Vec<String>,
}

fn directories(pages: &[Page], paths: &[PathBuf], content: &Path) -> BTreeMap<String, Directory> {
    let mut result = BTreeMap::new();
    for directory in paths.iter().map(|p| p.strip_prefix(content).unwrap()) {
        let key = directory.to_string_lossy().replace('\\', "/");
        result.insert(
            key.clone(),
            Directory {
                path: key,
                name: directory
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                index: None,
                pages: vec![],
                directories: vec![],
            },
        );
    }
    let keys: Vec<_> = result.keys().cloned().collect();
    for key in keys.into_iter().filter(|key| !key.is_empty()) {
        let parent = Path::new(&key).parent().unwrap().to_string_lossy();
        result
            .get_mut(parent.as_ref())
            .unwrap()
            .directories
            .push(key);
    }
    for (i, page) in pages.iter().enumerate() {
        let dir = result.get_mut(&page.directory).unwrap();
        if page
            .route
            .output
            .file_name()
            .is_some_and(|s| s == "index.html")
            && page.route.output.parent() == page.route.path.parent()
        {
            dir.index = Some(i);
        } else {
            dir.pages.push(i);
        }
    }
    result
}

fn discover<R: Resources>(
    vault: &Vault<R>,
    path: &Path,
    config: &SiteConfig,
    mounts: &crate::mounts::Mounts,
    include: &globset::GlobSet,
    exclude: &globset::GlobSet,
    sources: &mut Vec<PathBuf>,
    assets: &mut Vec<PathBuf>,
    directories: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    let logical = mounts.path(path).unwrap();
    if !logical.as_os_str().is_empty() && exclude.is_match(&logical) { return Ok(()); }
    directories.push(vault.resources().resolve(&config.content).join(&logical));
    for entry in vault.resources().entries(path)? {
        if entry
            .file_name()
            .is_some_and(|s| s.to_string_lossy().starts_with('.') || s == "Notist.toml" || s == "target" || s == "node_modules")
        {
            continue;
        }
        let excluded_roots = [&config.output, &config.templates, &config.static_dir, &config.data, &config.sass];
        if excluded_roots.into_iter().any(|p| entry.starts_with(vault.resources().resolve(p)))
            || config.theme.as_ref().filter(|p| !crate::themes::is_builtin(p)).is_some_and(|p| entry.starts_with(vault.resources().resolve(p))) {
            continue;
        }
        let logical = mounts.path(&entry).unwrap();
        if exclude.is_match(&logical) { continue; }
        match vault.resources().kind(&entry)? {
            Some(ResourceKind::Directory) => discover(vault, &entry, config, mounts, include, exclude, sources, assets, directories)?,
            Some(ResourceKind::File) if vault.supports(&entry) => { if include.is_match(&logical) { sources.push(entry) } },
            Some(ResourceKind::File) if !entry.extension().is_some_and(|s| s == "notc") => {
                assets.push(entry)
            }
            _ => {}
        }
    }
    Ok(())
}

fn issue(document: &Document, node: &Item, message: String) -> SourceDiagnostic {
    let span = node.origin.as_ref().map_or(node.span, |o| {
        notist::TextRange::new(o.start.into(), o.end.into())
    });
    let mut d = Diagnostic::new(Phase::Semantic, span, message);
    if let Some(origin) = &node.origin {
        d = d.at_source(&document.sources, origin.source);
    }
    diagnostic(&document.page.route.source, &document.source, d)
}

struct DocumentTarget {
    route: crate::Route,
    ids: BTreeSet<String>,
}
struct DocumentIndex {
    targets: Vec<DocumentTarget>,
    sources: HashMap<PathBuf, usize>,
    outputs: HashMap<PathBuf, usize>,
}
impl DocumentIndex {
    fn new(documents: &[Document]) -> Self {
        // Link destinations need routes and anchors, never another page's tree.
        // Own that small projection so rendered trees can be released in order.
        let targets: Vec<_> = documents
            .iter()
            .map(|document| DocumentTarget {
                route: document.page.route.clone(),
                ids: document.ids.clone(),
            })
            .collect();
        let sources: BTreeMap<_, _> = documents
            .iter()
            .enumerate()
            .map(|(i, document)| (document.page.route.source.clone(), i))
            .collect();
        let mut outputs = HashMap::new();
        // Keep source-path precedence, including ambiguous aliases, exactly as
        // the former ordered sources.values().find lookup did.
        for &i in sources.values() {
            let path = &targets[i].route.output;
            outputs.entry(path.clone()).or_insert(i);
            if path.file_name().is_some_and(|name| name == "index.html") {
                outputs.entry(path.parent().unwrap().into()).or_insert(i);
            }
        }
        Self {
            targets,
            // Ordering determines alias precedence above, not exact lookups.
            sources: sources.into_iter().collect(),
            outputs,
        }
    }
    fn find(&self, source: &Path, content: &Path) -> Option<&DocumentTarget> {
        self.sources
            .get(source)
            .copied()
            .or_else(|| {
                self.outputs
                    .get(source.strip_prefix(content).ok()?)
                    .copied()
            })
            .map(|i| &self.targets[i])
    }
}

fn resolve_link(
    document: &Document,
    target: &str,
    node: &Item,
    content: &Path,
    documents: &DocumentIndex,
    outputs: &BTreeSet<PathBuf>,
    mounts: &crate::mounts::Mounts,
    directory_urls: bool,
    warnings: &mut Vec<SourceDiagnostic>,
) -> Result<Option<String>, String> {
    if target.starts_with("//")
        || target
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .contains(':')
    {
        return Ok(None);
    }
    let (before, anchor) = target
        .split_once('#')
        .map_or((target, None), |(p, a)| (p, Some(a)));
    let (path, query) = before
        .split_once('?')
        .map_or((before, None), |(p, q)| (p, Some(q)));
    let path = percent_encoding::percent_decode_str(path)
        .decode_utf8()
        .map_err(|_| "invalid UTF-8 in link path")?;
    if path.contains(['\\', '\0']) {
        return Err("invalid local link path".into());
    }
    // Generated nodes retain their defining document as the reference base.
    let origin = node
        .origin
        .as_ref()
        .map(|o| document.sources.name(o.source).map(Path::new))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or(&document.page.route.source);
    let source = if path.is_empty() {
        Cow::Borrowed(origin)
    } else if path.starts_with('/') {
        Cow::Owned(normalize(&content.join(path.trim_start_matches('/'))))
    } else {
        Cow::Owned(normalize(&origin.parent().unwrap().join(path.as_ref())))
    };
    let source = mounts.resolve(&source);
    let logical = mounts.path(&source).map(|p| content.join(p));
    let destination_doc = documents.find(&source, content).or_else(|| logical.as_ref().and_then(|p| documents.find(p, content)));
    let destination = if let Some(destination) = destination_doc {
        if let Some(anchor) = anchor.filter(|a| !a.is_empty()) {
            let decoded = percent_encoding::percent_decode_str(anchor)
                .decode_utf8()
                .map_err(|_| "invalid UTF-8 in anchor")?;
            if !destination.ids.contains(decoded.as_ref()) {
                warnings.push(issue(document, node, format!(
                        "missing anchor `{anchor}` in `{}`",
                        destination.route.path.display()
                )));
            }
        }
        Cow::Borrowed(destination.route.output.as_path())
    } else {
        let Some(published) = mounts.path(&source) else {
            let mut warning = issue(document, node, format!("reference `{target}` is outside the content directory and configured mounts"));
            warning.diagnostic.code = Some(DiagnosticCode::OutsideVault);
            warnings.push(warning);
            return Ok(None);
        };
        if published.starts_with("_notist") { return Err("_notist is reserved for generated resources".into()); }
        if outputs.contains(&published) {
            Cow::Owned(published)
        } else {
            let index = published.join("index.html");
            if !outputs.contains(&index) {
                warnings.push(issue(document, node, format!("missing local reference `{target}` (not published)")));
                if source.extension().is_some_and(|s| s == "not" || s == "md") {
                    Cow::Owned(crate::Route::plan(content.join(&published), content, directory_urls).map_err(|e| e.to_string())?.output)
                } else { Cow::Owned(published) }
            } else {
                Cow::Owned(index)
            }
        }
    };
    let mut url = if path.is_empty() && destination.as_ref() == document.page.route.output {
        String::new()
    } else {
        if directory_urls && destination.file_name().is_some_and(|s| s == "index.html") {
            relative_url(&document.page.route.output, destination.parent().unwrap(), true)
        } else { relative_url(&document.page.route.output, &destination, false) }
    };
    if let Some(query) = query {
        url.push('?');
        url.push_str(query);
    }
    if let Some(anchor) = anchor {
        let decoded = percent_encoding::percent_decode_str(anchor)
            .decode_utf8()
            .map_err(|_| "invalid UTF-8 in anchor")?;
        url.push('#');
        url.push_str(&fragment(&decoded));
    }
    if url.is_empty() {
        url.push_str("./");
    }
    Ok(Some(url))
}

fn resolve_embed(
    document: &Document,
    path: &notist::ResourcePath,
    node: &Item,
    content: &Path,
    documents: &DocumentIndex,
    outputs: &BTreeSet<PathBuf>,
    mounts: &crate::mounts::Mounts,
    directory_urls: bool,
) -> Result<String, String> {
    let source = mounts.resolve(Path::new(&path.logical_path()));
    let logical = mounts.path(&source).map(|p| content.join(p));
    let destination = documents
        .find(&source, content)
        .or_else(|| logical.as_ref().and_then(|p| documents.find(p, content)));
    let output = if let Some(destination) = destination {
        if let Some(anchor) =
            notist::embed::string(&node.fields(), "fragment").filter(|anchor| !anchor.is_empty())
            && !destination.ids.contains(anchor)
        {
            return Err(format!(
                "missing anchor `#{anchor}` in `{}`",
                destination.route.path.display()
            ));
        }
        Cow::Borrowed(destination.route.output.as_path())
    } else {
        let published = mounts.path(&source).ok_or_else(|| {
            format!(
                "embedded resource `{}` is outside the content directory",
                path.path()
            )
        })?;
        if outputs.contains(&published) {
            Cow::Owned(published)
        } else {
            let index = published.join("index.html");
            if !outputs.contains(&index) {
                return Err(format!("missing embedded resource `{}`", path.path()));
            }
            Cow::Owned(index)
        }
    };
    if output.as_ref() == document.page.route.output {
        Ok(String::new())
    } else {
        Ok(
            if directory_urls && output.file_name().is_some_and(|s| s == "index.html") {
                relative_url(&document.page.route.output, output.parent().unwrap(), true)
            } else {
                relative_url(&document.page.route.output, &output, false)
            },
        )
    }
}

fn image(node: &Item, url: &str) -> Option<String> {
    if !notist_html::is_safe_url(url) {
        return None;
    }
    let extension = Path::new(url.split(['?', '#']).next()?)
        .extension()?
        .to_str()?;
    if ![
        "png", "jpg", "jpeg", "gif", "svg", "webp", "avif", "apng", "bmp", "ico",
    ]
    .iter()
    .any(|e| extension.eq_ignore_ascii_case(e))
    {
        return None;
    }
    let alt = match node.field("description") {
        Some(Value::Str(s)) => s.as_str(),
        _ => "",
    };
    Some(format!(
        "<img src=\"{}\" alt=\"{}\">",
        notist_html::escape_attribute(url),
        notist_html::escape_attribute(alt)
    ))
}

/// Rendering uses only immutable prepared values. Resource reads, document
/// hooks, evaluation and publication remain ordered host operations.
fn render_templates(
    engine: &tera::Tera,
    config: &SiteConfig,
    pages: &[Page],
    values: &TemplateContext,
    components: &BTreeMap<PathBuf, String>,
    highlight_css: &Path,
    workers: usize,
) -> Result<Vec<Vec<u8>>, Error> {
    let render = |start: usize, end: usize| {
        let mut context = values.context.clone();
        let mut current_directory: Option<&str> = None;
        let mut output = Vec::with_capacity(end - start);
        for (i, page) in pages.iter().enumerate().take(end).skip(start) {
            let template = page
                .attrs
                .get("template")
                .and_then(|v| v.as_str())
                .unwrap_or(&config.template);
            if current_directory != Some(page.directory.as_str()) {
                context.insert_value("directory", values.directories[&page.directory].clone());
                current_directory = Some(&page.directory);
            }
            context.insert_value("page", values.pages[i].clone());
            context.insert(
                "assets",
                &PageAssets {
                    components: components.get(&page.route.path),
                    highlight: relative_url(&page.route.output, highlight_css, false),
                    styles: config
                        .styles
                        .iter()
                        .map(|p| relative_url(&page.route.output, p, false))
                        .collect(),
                    scripts: config
                        .scripts
                        .iter()
                        .map(|p| relative_url(&page.route.output, p, false))
                        .collect(),
                },
            );
            let html = engine
                .render(template, &context)
                .map_err(|e| Error::Message(format!("{}: {e}", page.route.path.display())))?;
            output.push(html.into_bytes());
        }
        Ok::<_, Error>(output)
    };
    let workers = workers.clamp(1, 8).min(pages.len().max(1));
    if workers == 1 {
        return render(0, pages.len());
    }
    std::thread::scope(|scope| {
        let chunk = pages.len().div_ceil(workers);
        let render = &render;
        let handles: Vec<_> = (0..pages.len())
            .step_by(chunk)
            .map(|start| scope.spawn(move || render(start, (start + chunk).min(pages.len()))))
            .collect();
        let mut output = Vec::with_capacity(pages.len());
        // Ordered joins preserve the first page error and deterministic output.
        for handle in handles {
            let rendered = handle
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
            output.extend(rendered);
        }
        Ok(output)
    })
}

pub(crate) fn build<R: Resources>(
    vault: &mut Vault<R>,
    config: &SiteConfig,
    extensions: &[Box<dyn crate::extensions::ExtensionFactory>],
    recover: bool,
) -> Result<BuildOutput, Error> {
    let config = &{
        let mut config = config.clone();
        let mounts = crate::mounts::Mounts::new(vault.resources(), &config)?;
        let content = vault.resources().resolve(&config.content);
        for path in config.styles.iter_mut().chain(&mut config.scripts) {
            let source = mounts.resolve(&normalize(&content.join(&*path)));
            *path = mounts.path(&source).ok_or_else(|| {
                Error::Message(
                    "additional site assets must be within content or configured mounts".into(),
                )
            })?;
        }
        config
    };
    config.validate()?;
    let mut extensions: Vec<_> = extensions.iter().map(|e| e.create()).collect();
    extensions.extend(crate::extensions::configured(config)?);
    let content = vault.resources().resolve(&config.content);
    let mounts = crate::mounts::Mounts::new(vault.resources(), config)?;
    let output = vault.resources().resolve(&config.output);
    if content.starts_with(&output) || vault.resources().root().starts_with(&output) {
        return Err(Error::Message(
            "site output and content sources overlap".into(),
        ));
    }
    let mut inputs = vec![
        config.templates.clone(),
        config.static_dir.clone(),
        config.data.clone(),
        config.sass.clone(),
    ];
    if let Some(theme) = &config.theme {
        if !crate::themes::is_builtin(theme) {
            inputs.push(theme.clone());
        }
    }
    for input in inputs {
        let input = vault.resources().resolve(&input);
        if input.starts_with(&output) || output.starts_with(&input) || content.starts_with(&input) {
            return Err(Error::Message(format!(
                "site input `{}` overlaps content or output",
                input.display()
            )));
        }
    }
    let content_kind = vault.resources().kind(&content)?;
    let generated_only =
        content_kind.is_none() && (!extensions.is_empty() || !config.extensions.is_empty());
    if content_kind != Some(ResourceKind::Directory) && !generated_only {
        return Err(Error::Message(format!(
            "content directory `{}` does not exist",
            content.display()
        )));
    }
    let mut sources = vec![];
    let discovery = notist::progress::Span::enter(|| "Discover documents".into());
    let mut content_assets = vec![];
    let mut source_dirs = vec![];
    let globs = |patterns: &[String]| -> Result<globset::GlobSet, Error> {
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(
                globset::GlobBuilder::new(pattern)
                    .literal_separator(true)
                    .build()
                    .map_err(|e| Error::Message(format!("invalid site glob `{pattern}`: {e}")))?,
            );
        }
        builder.build().map_err(|e| Error::Message(e.to_string()))
    };
    let include = globs(&config.include)?;
    let exclude = globs(&config.exclude)?;
    if generated_only {
        source_dirs.push(content.clone());
    } else {
        for (root, _) in mounts.roots() {
            discover(
                vault,
                root,
                config,
                &mounts,
                &include,
                &exclude,
                &mut sources,
                &mut content_assets,
                &mut source_dirs,
            )?;
        }
    }
    let mut input = crate::extensions::CompileInput {
        documents: sources
            .into_iter()
            .map(|source| Ok((mounts.path(&source).unwrap(), vault.source(&source)?)))
            .collect::<Result<_, Error>>()?,
    };
    discovery.finish();
    let routes = notist::progress::run(
        || "Run site extensions and plan routes".into(),
        || crate::extensions::before_compile(vault, config, &mut extensions, &mut input),
    )?;
    let mut source_dirs: BTreeSet<_> = source_dirs.into_iter().collect();
    // Mounts may introduce virtual ancestors without documents of their own.
    for directory in source_dirs.clone() {
        for parent in directory.ancestors().skip(1) {
            if !parent.starts_with(&content) {
                break;
            }
            source_dirs.insert(parent.into());
        }
    }
    for path in input.documents.keys() {
        for parent in content.join(path).parent().unwrap().ancestors() {
            source_dirs.insert(parent.into());
            if parent == content {
                break;
            }
        }
    }
    let source_dirs: Vec<_> = source_dirs.into_iter().collect();
    let mut documents = vec![];
    let mut errors = vec![];
    let mut outputs: BTreeSet<_> = routes.iter().map(|route| route.output.clone()).collect();
    let mut analyzed = Vec::new();
    let analysis_stage = notist::progress::Span::enter(|| "Analyze documents".into());
    for ((_, text), route) in input.documents.into_iter().zip(routes) {
        let (analysis, handle) = vault.analyze_document_with_resource_root(
            &route.source,
            &text,
            mounts
                .roots()
                .find(|(root, _)| route.source.starts_with(root))
                .map(|(root, _)| root)
                .unwrap_or(&content),
        )?;
        errors.extend(
            analysis
                .diagnostics()
                .iter()
                .filter(|d| d.code != Some(DiagnosticCode::OutsideVault))
                .cloned()
                .map(|d| diagnostic(&route.source, &text, d)),
        );
        analyzed.push((route, text, analysis, handle));
    }
    analysis_stage.finish();
    // Fix every generated/edited document source in the shared input view before
    // any page can consume another document as text, regardless of route order.
    let mut handles = Vec::with_capacity(analyzed.len());
    let evaluation_stage = notist::progress::Span::enter(|| "Evaluate documents".into());
    for (route, text, analysis, handle) in analyzed {
        let document =
            notist::progress::Span::enter(|| format!("Document {}", route.path.display()));
        let evaluation = vault.evaluate_document(&handle, &analysis)?;
        errors.extend(
            evaluation
                .diagnostics
                .into_iter()
                .map(|d| diagnostic(&route.source, &text, d)),
        );
        let mut prepared = Document::new(route, evaluation.root, text, evaluation.sources)?;
        if prepared.page.route.url == "/"
            && prepared.page.headings.is_empty()
            && !prepared.page.attrs.contains_key("title")
        {
            prepared.page.title = config.title.clone();
        }
        documents.push(prepared);
        handles.push(handle);
        document.finish();
    }
    let mut diagnostics = vec![];
    if recover {
        let (local, fatal): (Vec<_>, Vec<_>) =
            errors.into_iter().partition(|e| e.diagnostic.recovered);
        diagnostics = local;
        errors = fatal;
    }
    if !errors.is_empty() {
        return Err(Error::Diagnostics(errors));
    }
    evaluation_stage.finish();
    let asset_stage = notist::progress::Span::enter(|| "Prepare assets and Sass".into());
    let mut assets = Assets::new(output);
    for document in &documents {
        assets.reserve_generated(document.page.route.output.clone())?;
    }
    outputs.extend(assets::prepare(
        vault.resources(),
        config,
        &content_assets,
        &mounts,
        &mut assets,
    )?);
    let highlight_css = PathBuf::from("_notist/highlight.css");
    assets.generated(
        highlight_css.clone(),
        notist_html::highlight::CSS.as_bytes().to_vec(),
    )?;
    for path in config.styles.iter().chain(&config.scripts) {
        if !crate::extensions::valid_path(path) || path.starts_with("_notist") {
            return Err(Error::Message(
                "additional site assets must be content-relative paths outside _notist".into(),
            ));
        }
        if !outputs.contains(path) {
            let source = mounts.resolve(&content.join(path));
            if source.starts_with(vault.resources().resolve(&config.output)) {
                return Err(Error::Message(
                    "additional site asset and output overlap".into(),
                ));
            }
            if vault.resources().kind(&source)? != Some(ResourceKind::File) {
                return Err(Error::Message(format!(
                    "additional site asset `{}` is not a file",
                    path.display()
                )));
            }
            assets.copy(vault.resources(), &source, path)?;
            outputs.insert(path.clone());
        }
    }
    asset_stage.finish();
    let by_source = DocumentIndex::new(&documents);
    let mut warnings = vec![];
    let mut pages = vec![];
    let mut components = BTreeMap::new();
    let rendering = notist::progress::Span::enter(|| "Resolve links and render HTML".into());
    for (document, handle) in documents.into_iter().zip(handles) {
        let page_progress = notist::progress::Span::enter(|| {
            format!("Render {}", document.page.route.path.display())
        });
        let mut resolved = BTreeMap::new();
        let mut resource_urls = BTreeMap::new();
        for node in notist::embed::references(&document.root) {
            if node.is_error() {
                continue;
            }
            // Only explicit references publish files when bulk copying is disabled.
            let referenced = match node.field("target") {
                Some(Value::Path(path)) => Some(PathBuf::from(path.logical_path())),
                Some(Value::Str(url)) if !notist::embed::is_embed(node) => {
                    let origin = node
                        .origin
                        .as_ref()
                        .and_then(|o| document.sources.name(o.source).ok())
                        .map(Path::new)
                        .unwrap_or(&document.page.route.source);
                    let local = url.split(['?', '#']).next().unwrap_or("");
                    if local.is_empty()
                        || local.starts_with("//")
                        || local.split('/').next().unwrap_or("").contains(':')
                    {
                        None
                    } else {
                        percent_encoding::percent_decode_str(local)
                            .decode_utf8()
                            .ok()
                            .map(|p| {
                                if p.starts_with('/') {
                                    normalize(&content.join(p.trim_start_matches('/')))
                                } else {
                                    normalize(&origin.parent().unwrap().join(p.as_ref()))
                                }
                            })
                    }
                }
                _ => None,
            };
            if let Some(source) = referenced {
                let source = mounts.resolve(&source);
                if let Some(destination) = mounts.path(&source) {
                    if destination.starts_with("_notist") {
                        errors.push(issue(
                            &document,
                            node,
                            "_notist is reserved for generated resources".into(),
                        ));
                        continue;
                    }
                    if !outputs.contains(&destination)
                        && !vault.supports(&source)
                        && !source.extension().is_some_and(|s| s == "notc")
                        && !destination
                            .components()
                            .any(|p| p.as_os_str().to_string_lossy().starts_with('.'))
                        && vault.resources().kind(&source)? == Some(ResourceKind::File)
                    {
                        assets.copy(vault.resources(), &source, &destination)?;
                        outputs.insert(destination);
                    }
                }
            }
            if notist::embed::is_embed(node) || matches!(node.field("target"), Some(Value::Path(_)))
            {
                if let Some(Value::Path(path)) = node.field("target") {
                    match resolve_embed(
                        &document,
                        path,
                        node,
                        &content,
                        &by_source,
                        &outputs,
                        &mounts,
                        config.directory_urls,
                    ) {
                        Ok(url) => {
                            resource_urls.insert(path.clone(), url);
                        }
                        Err(message) => errors.push(issue(&document, node, message)),
                    }
                }
                continue;
            }
            let Some(Value::Str(target)) = node.field("target") else {
                continue;
            };
            match resolve_link(
                &document,
                target,
                node,
                &content,
                &by_source,
                &outputs,
                &mounts,
                config.directory_urls,
                &mut warnings,
            ) {
                Ok(Some(url)) => {
                    resolved.insert(
                        (node.origin.as_ref().map(|o| o.source), target.clone()),
                        url,
                    );
                }
                Ok(None) => {}
                Err(message) => {
                    let d = issue(&document, node, message);
                    errors.push(d);
                }
            }
        }
        let lookup = |node: &Item, url: &str| {
            resolved
                .get(&(node.origin.as_ref().map(|o| o.source), url.into()))
                .cloned()
        };
        let (registry, environment) = vault.html_registry_for_document(&handle, &document.root)?;
        let rendered = notist_html::Renderer::from_registry(registry)
            .with_sources(&document.sources)
            .with_code_renderer(notist_html::highlight::code)
            .with_url_resolver(lookup)
            .with_resource_resolver(|_, path| resource_urls.get(path).cloned())
            .with_embed_renderer(|node| {
                let base = match node.field("target")? {
                    Value::Path(path) => resource_urls.get(path)?.as_str(),
                    Value::Str(url) => url.as_str(),
                    _ => return None,
                };
                let url = notist::embed::append_suffix(base, &node.fields()).ok()?;
                image(node, &url)
            })
            .render_with_diagnostics(&document.root);
        errors.extend(
            rendered
                .diagnostics
                .into_iter()
                .map(|d| diagnostic(&document.page.route.source, &document.source, d)),
        );
        let component_path = PathBuf::from("_notist/pages")
            .join(&document.page.route.path)
            .with_extension("js");
        let registrations = assets.components(
            vault.resources(),
            &environment,
            &rendered.used_components,
            &component_path,
        )?;
        if !registrations.is_empty() {
            components.insert(
                document.page.route.path.clone(),
                relative_url(&document.page.route.output, &component_path, false),
            );
            assets.generated(component_path, registrations.into_bytes())?;
        }
        let mut page = document.page;
        page.html = rendered.html;
        pages.push(page);
        page_progress.finish();
    }
    if !errors.is_empty() {
        return Err(Error::Diagnostics(errors));
    }
    rendering.finish();
    let mut template_input = crate::extensions::TemplateInput {
        pages: &pages,
        data: assets::data(vault.resources(), &vault.resources().resolve(&config.data))?,
    };
    crate::extensions::before_template(
        vault.resources(),
        config,
        &mut extensions,
        &mut template_input,
    )?;
    let data = template_input.data;
    crate::extensions::finish(&mut extensions)?;
    let templates_stage =
        notist::progress::Span::enter(|| format!("Render templates ({} pages)", pages.len()));
    let engine = templates::load(vault.resources(), config)?;
    let dirs = directories(&pages, &source_dirs, &content);
    let values = TemplateContext::new(config, &pages, &dirs, &data)?;
    let workers = if pages.len() >= 1024 {
        std::thread::available_parallelism().map_or(1, usize::from)
    } else {
        1
    };
    let rendered = render_templates(
        &engine,
        config,
        &pages,
        &values,
        &components,
        &highlight_css,
        workers,
    )?;
    for (page, bytes) in pages.iter().zip(rendered) {
        assets.write_reserved(&page.route.output, bytes)?;
    }
    templates_stage.finish();
    Ok(BuildOutput {
        files: assets.finish()?,
        pages,
        warnings,
        diagnostics,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Route;

    #[test]
    fn destination_index_owns_only_metadata_and_preserves_alias_precedence() {
        let content = Path::new("/site/content");
        let mut documents: Vec<_> = [
            ("a.not", "not"),
            ("a.md", "markdown"),
            ("nested/index.not", "index"),
        ]
        .into_iter()
        .map(|(source, id)| {
            let mut root = Item::new(notist::Ctor::Doc, Default::default());
            root.attrs.insert("id", Value::Str(id.into()));
            Document::new(
                Route::new(content.join(source), content).unwrap(),
                root,
                String::new(),
                Default::default(),
            )
            .unwrap()
        })
        .collect();
        let index = DocumentIndex::new(&documents);
        documents.reverse();
        let reversed = DocumentIndex::new(&documents);
        drop(documents);
        for index in [index, reversed] {
            let alias = index.find(&content.join("a.html"), content).unwrap();
            assert_eq!(alias.route.source, content.join("a.md"));
            assert!(alias.ids.contains("markdown"));
            let direct = index.find(&content.join("a.not"), content).unwrap();
            assert!(direct.ids.contains("not"));
            let directory = index.find(&content.join("nested"), content).unwrap();
            assert_eq!(directory.route.output, Path::new("nested/index.html"));
            assert!(directory.ids.contains("index"));
        }
    }

    fn field<'a>(value: &'a tera::Value, key: &str) -> &'a tera::Value {
        value
            .as_map()
            .unwrap()
            .get(&tera::value::Key::from(key.to_owned()))
            .unwrap()
    }

    #[test]
    fn template_context_shares_page_and_directory_maps_without_losing_data() {
        let content = Path::new("/site/content");
        let pages: Vec<_> = ["index.not", "a.not", "folder/index.not"]
            .into_iter()
            .map(|path| Page {
                route: Route::new(content.join(path), content).unwrap(),
                title: path.into(),
                attrs: BTreeMap::from([(
                    "nested".into(),
                    serde_json::json!({"enabled": true, "values": [42, null, "<&>"]}),
                )]),
                headings: vec![],
                html: "<strong>body</strong>".into(),
                directory: Path::new(path)
                    .parent()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                ancestors: vec![],
            })
            .collect();
        let dirs = directories(
            &pages,
            &[
                content.into(),
                content.join("empty"),
                content.join("folder"),
            ],
            content,
        );
        let mut values =
            TemplateContext::new(&SiteConfig::default(), &pages, &dirs, &BTreeMap::new()).unwrap();
        values.context.insert_value("page", values.pages[0].clone());
        values
            .context
            .insert_value("directory", values.directories[""].clone());
        let site = values.context.get("site").unwrap();
        let shared_pages = field(site, "pages").as_array().unwrap();
        let root = field(field(site, "directories"), "");
        for (left, right) in [
            (&values.pages[0], &shared_pages[0]),
            (&values.pages[0], field(root, "index")),
            (&values.pages[0], values.context.get("page").unwrap()),
            (
                &values.pages[1],
                &field(root, "pages").as_array().unwrap()[0],
            ),
            (
                &values.pages[2],
                field(field(field(site, "directories"), "folder"), "index"),
            ),
            (root, values.context.get("directory").unwrap()),
        ] {
            assert!(std::ptr::eq(
                left.as_map().unwrap(),
                right.as_map().unwrap()
            ));
        }
        let empty = field(field(site, "directories"), "empty");
        assert!(field(empty, "index").is_none());
        assert!(field(empty, "pages").as_array().unwrap().is_empty());
        for (page, value) in pages.iter().zip(shared_pages) {
            assert_eq!(
                serde_json::to_value(page).unwrap(),
                serde_json::to_value(value).unwrap()
            );
        }
    }

    #[test]
    fn template_context_accepts_an_empty_generated_site() {
        let values = TemplateContext::new(
            &SiteConfig::default(),
            &[],
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        let site = values.context.get("site").unwrap();
        assert!(field(site, "pages").as_array().unwrap().is_empty());
        assert!(field(site, "directories").as_map().unwrap().is_empty());
        assert!(field(site, "data").as_map().unwrap().is_empty());
    }
    #[test]
    fn parallel_templates_preserve_values_order_and_first_error() {
        let content = Path::new("/site/content");
        let pages: Vec<_> = (0..24)
            .map(|i| Page {
                route: Route::new(content.join(format!("p{i}.not")), content).unwrap(),
                title: format!("page-{i}"),
                attrs: BTreeMap::new(),
                headings: vec![],
                html: format!("<strong>{i}</strong>"),
                directory: String::new(),
                ancestors: vec![],
            })
            .collect();
        let config = SiteConfig::default();
        let dirs = directories(&pages, &[content.into()], content);
        let values = TemplateContext::new(&config, &pages, &dirs, &BTreeMap::new()).unwrap();
        let mut engine = tera::Tera::default();
        engine.add_raw_template("page", "{{page.title}}|{{page.html | safe}}|{{directory.pages | length}}|{{site.pages | length}}").unwrap();
        let components = BTreeMap::new();
        let highlight = Path::new("_notist/highlight.css");
        let serial =
            render_templates(&engine, &config, &pages, &values, &components, highlight, 1).unwrap();
        let parallel =
            render_templates(&engine, &config, &pages, &values, &components, highlight, 4).unwrap();
        assert_eq!(serial, parallel);
        assert!(
            String::from_utf8(parallel[7].clone())
                .unwrap()
                .starts_with("page-7|<strong>7</strong>|24|24")
        );
        let mut invalid = pages;
        for index in [3, 18] {
            invalid[index]
                .attrs
                .insert("template".into(), serde_json::json!("missing"));
        }
        let serial = render_templates(
            &engine,
            &config,
            &invalid,
            &values,
            &components,
            highlight,
            1,
        )
        .unwrap_err()
        .to_string();
        let parallel = render_templates(
            &engine,
            &config,
            &invalid,
            &values,
            &components,
            highlight,
            4,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(serial, parallel);
        assert!(serial.contains("p3.not"));
    }
}
