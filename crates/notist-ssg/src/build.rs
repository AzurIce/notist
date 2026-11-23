use crate::{
    Error, Page, SiteConfig, assets,
    document::diagnostic,
    prepared::{PreparedPage, Reference},
    templates,
};
use notc::{
    Decl, Module, ProjectDiagnosticKind, check_parsed_modules, format_type_error, parse,
    parse_document, parse_markdown_document, resolve_import_path,
};
use notist::{
    Diagnostic, DiagnosticCode, Phase, Resources, SourceDiagnostic, TextRange, Vault,
    publication::{
        Assets,
        url::{fragment, relative_url},
    },
    resources::{ResourceKind, normalize},
};
use serde::Serialize;
use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    path::{Component, Path, PathBuf},
    sync::Arc,
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
    if !logical.as_os_str().is_empty() && exclude.is_match(&logical) {
        return Ok(());
    }
    directories.push(vault.resources().resolve(&config.content).join(&logical));
    for entry in vault.resources().entries(path)? {
        if entry.file_name().is_some_and(|s| {
            s.to_string_lossy().starts_with('.')
                || s == "Notist.toml"
                || s == "target"
                || s == "node_modules"
        }) {
            continue;
        }
        let excluded_roots = [
            &config.output,
            &config.templates,
            &config.static_dir,
            &config.data,
            &config.sass,
        ];
        if excluded_roots
            .into_iter()
            .any(|p| entry.starts_with(vault.resources().resolve(p)))
            || config
                .theme
                .as_ref()
                .filter(|p| !crate::themes::is_builtin(p))
                .is_some_and(|p| entry.starts_with(vault.resources().resolve(p)))
        {
            continue;
        }
        let logical = mounts.path(&entry).unwrap();
        if exclude.is_match(&logical) {
            continue;
        }
        match vault.resources().kind(&entry)? {
            Some(ResourceKind::Directory) => discover(
                vault,
                &entry,
                config,
                mounts,
                include,
                exclude,
                sources,
                assets,
                directories,
            )?,
            Some(ResourceKind::File) if vault.supports(&entry) => {
                if include.is_match(&logical) {
                    sources.push(entry)
                }
            }
            Some(ResourceKind::File) if !entry.extension().is_some_and(|s| s == "notc") => {
                assets.push(entry)
            }
            _ => {}
        }
    }
    Ok(())
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
    fn new<'a>(
        documents: impl IntoIterator<Item = (&'a crate::Route, &'a BTreeSet<String>)>,
    ) -> Self {
        // Link destinations need routes and anchors, never another page's tree.
        // Own that small projection so rendered trees can be released in order.
        let targets: Vec<_> = documents
            .into_iter()
            .map(|(route, ids)| DocumentTarget {
                route: route.clone(),
                ids: ids.clone(),
            })
            .collect();
        let sources: BTreeMap<_, _> = targets
            .iter()
            .enumerate()
            .map(|(i, document)| (document.route.source.clone(), i))
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
    route: &crate::Route,
    target: &str,
    node: &Reference,
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
    let origin = &node.origin;
    let source = if path.is_empty() {
        Cow::Borrowed(origin.as_path())
    } else if path.starts_with('/') {
        Cow::Owned(normalize(&content.join(path.trim_start_matches('/'))))
    } else {
        Cow::Owned(normalize(&origin.parent().unwrap().join(path.as_ref())))
    };
    let source = mounts.resolve(&source);
    let logical = mounts.path(&source).map(|p| content.join(p));
    let destination_doc = documents
        .find(&source, content)
        .or_else(|| logical.as_ref().and_then(|p| documents.find(p, content)));
    let destination = if let Some(destination) = destination_doc {
        if let Some(anchor) = anchor.filter(|a| !a.is_empty()) {
            let decoded = percent_encoding::percent_decode_str(anchor)
                .decode_utf8()
                .map_err(|_| "invalid UTF-8 in anchor")?;
            if !destination.ids.contains(decoded.as_ref()) {
                warnings.push(node.issue(format!(
                    "missing anchor `{anchor}` in `{}`",
                    destination.route.path.display()
                )));
            }
        }
        Cow::Borrowed(destination.route.output.as_path())
    } else {
        let Some(published) = mounts.path(&source) else {
            let mut warning = node.issue(format!(
                "reference `{target}` is outside the content directory and configured mounts"
            ));
            warning.diagnostic.code = Some(DiagnosticCode::OutsideVault);
            warnings.push(warning);
            return Ok(None);
        };
        if published.starts_with("_notist") {
            return Err("_notist is reserved for generated resources".into());
        }
        if outputs.contains(&published) {
            Cow::Owned(published)
        } else {
            let index = published.join("index.html");
            if !outputs.contains(&index) {
                warnings.push(node.issue(format!(
                    "missing local reference `{target}` (not published)"
                )));
                if source.extension().is_some_and(|s| s == "not" || s == "md") {
                    Cow::Owned(
                        crate::Route::plan(content.join(&published), content, directory_urls)
                            .map_err(|e| e.to_string())?
                            .output,
                    )
                } else {
                    Cow::Owned(published)
                }
            } else {
                Cow::Owned(index)
            }
        }
    };
    let mut url = if path.is_empty() && destination.as_ref() == route.output {
        String::new()
    } else {
        if directory_urls && destination.file_name().is_some_and(|s| s == "index.html") {
            relative_url(&route.output, destination.parent().unwrap(), true)
        } else {
            relative_url(&route.output, &destination, false)
        }
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

/// Rendering uses only immutable prepared values. Resource reads, document
/// hooks and publication remain ordered host operations.
fn render_templates(
    engine: &tera::Tera,
    config: &SiteConfig,
    pages: &[Page],
    values: &TemplateContext,
    components: &BTreeMap<PathBuf, String>,
    highlight_css: &Path,
    executor: &notist::Executor,
) -> Result<Vec<Vec<u8>>, Error> {
    let render = |start: usize, chunk: &[Page]| {
        let mut context = values.context.clone();
        let mut current_directory: Option<&str> = None;
        let mut output = Vec::with_capacity(chunk.len());
        for (offset, page) in chunk.iter().enumerate() {
            let i = start + offset;
            let timing = notist::progress::Span::stage("site.template.page", || {
                format!("Template {}", page.route.path.display())
            });
            timing.attribute("source", || page.route.source.display().to_string());
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
            timing.finish();
        }
        Ok::<_, Error>(output)
    };
    // Chunk sizing is a batching choice, not an independent concurrency limit.
    let chunk = pages.len().div_ceil(executor.jobs()).max(128);
    let chunks = executor.map_chunks(pages, chunk, render);
    let mut output = Vec::with_capacity(pages.len());
    // Ordered collection preserves the first page error and deterministic output.
    for chunk in chunks {
        output.extend(chunk?);
    }
    Ok(output)
}

pub(crate) fn build<R: Resources + Send>(
    vault: &mut Vault<R>,
    config: &SiteConfig,
    extensions: &[Box<dyn crate::extensions::ExtensionFactory>],
    recover: bool,
    executor: &notist::Executor,
) -> Result<BuildOutput, Error> {
    notist::progress::run_stage(
        "site.build",
        || "Assemble site".into(),
        || {
            let config = &{
                let mut config = config.clone();
                let mounts = crate::mounts::Mounts::new(vault.resources(), &config)?;
                let content = vault.resources().resolve(&config.content);
                for path in config.styles.iter_mut().chain(&mut config.scripts) {
                    let source = mounts.resolve(&normalize(&content.join(&*path)));
                    *path = mounts.path(&source).ok_or_else(|| {
                        Error::Message(
                            "additional site assets must be within content or configured mounts"
                                .into(),
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
                if input.starts_with(&output)
                    || output.starts_with(&input)
                    || content.starts_with(&input)
                {
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
            let discovery =
                notist::progress::Span::stage("site.discover", || "Discover documents".into());
            let mut content_assets = vec![];
            let mut source_dirs = vec![];
            let globs = |patterns: &[String]| -> Result<globset::GlobSet, Error> {
                let mut builder = globset::GlobSetBuilder::new();
                for pattern in patterns {
                    builder.add(
                        globset::GlobBuilder::new(pattern)
                            .literal_separator(true)
                            .build()
                            .map_err(|e| {
                                Error::Message(format!("invalid site glob `{pattern}`: {e}"))
                            })?,
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
                    .map(|source| {
                        let relative = mounts.path(&source).unwrap();
                        let path = relative_path::RelativePathBuf::from_path(&relative).map_err(
                            |error| {
                                Error::Message(format!(
                                    "invalid site document path `{}`: {error}",
                                    relative.display()
                                ))
                            },
                        )?;
                        Ok((PathBuf::from(path.into_string()), vault.source(&source)?))
                    })
                    .collect::<Result<_, Error>>()?,
            };
            discovery.finish();
            let routes = notist::progress::run_stage(
                "site.routes",
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
            let mut outputs: BTreeSet<_> =
                routes.iter().map(|route| route.output.clone()).collect();
            let analysis_stage =
                notist::progress::Span::stage("site.analyze", || "Compile documents with notc".into());
            let (vroot, doc_entries, checked_project, compile_errors) =
                compile_notc_site(vault.resources(), &routes, input.documents)?;
            errors.extend(compile_errors);
            analysis_stage.finish();

            let diagnostics = vec![];
            let _ = recover;
            if !errors.is_empty() {
                return Err(Error::Diagnostics(errors));
            }

            let evaluation_stage = notist::progress::Span::stage("site.evaluate", || {
                "Evaluate and prepare pages".into()
            });
            if let Some(project) = checked_project {
                for (route, virtual_path, source_text) in doc_entries {
                    let block = project.eval_document(&virtual_path).map_err(|err| {
                        Error::Message(format!(
                            "{}: runtime error evaluating document: {err:?}",
                            route.path.display()
                        ))
                    })?;
                    let prepared = notist::progress::run_stage(
                        "site.document.prepare",
                        || format!("Prepare {}", route.path.display()),
                        || PreparedPage::new(route.clone(), source_text, block, &config.title),
                    )?;
                    documents.push(prepared);
                }
            }
            evaluation_stage.finish();
            let asset_stage =
                notist::progress::Span::stage("site.assets", || "Prepare assets and Sass".into());
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
                        "additional site assets must be content-relative paths outside _notist"
                            .into(),
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
            let by_source = DocumentIndex::new(documents.iter().map(|d| (&d.page.route, &d.ids)));
            for document in &mut documents {
                document.ids.clear();
            }
            let mut warnings = vec![];
            let mut pages = vec![];
            let mut components: BTreeMap<PathBuf, String> = BTreeMap::new();
            let mut published_component_files: BTreeSet<PathBuf> = BTreeSet::new();
            let rendering = notist::progress::Span::stage("site.render", || {
                "Resolve links and render HTML".into()
            });
            for document in documents {
                let page_progress = notist::progress::Span::stage("site.document.render", || {
                    format!("Render {}", document.page.route.path.display())
                });
                page_progress.attribute("source", || {
                    document.page.route.source.display().to_string()
                });
                let links = notist::progress::Span::stage("site.links", || {
                    "Resolve document links and resources".into()
                });
                let mut resolved = vec![None; document.references.len()];
                for (key, node) in document.references.iter().enumerate() {
                    let target = &node.target;
                    let local = target.split(['?', '#']).next().unwrap_or("");
                    let referenced = if local.is_empty()
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
                                    normalize(&node.origin.parent().unwrap().join(p.as_ref()))
                                }
                            })
                    };
                    if let Some(source) = referenced {
                        let source = mounts.resolve(&source);
                        if let Some(destination) = mounts.path(&source) {
                            if destination.starts_with("_notist") {
                                errors.push(
                                    node.issue(
                                        "_notist is reserved for generated resources".into(),
                                    ),
                                );
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
                    let mut local_warnings = Vec::new();
                    match resolve_link(
                        &document.page.route,
                        target,
                        node,
                        &content,
                        &by_source,
                        &outputs,
                        &mounts,
                        config.directory_urls,
                        &mut local_warnings,
                    ) {
                        Ok(Some(url)) => {
                            resolved[key] = Some(url);
                        }
                        Ok(None) => {}
                        Err(message) => {
                            errors.push(node.issue(message));
                        }
                    }
                    if node.is_image {
                        errors.extend(local_warnings);
                    } else {
                        warnings.extend(local_warnings);
                    }
                }
                links.finish();
                // SSG-only Web Component convention: for any bodyless `Custom` node
                // declared in `<module>.notc`, check if `<module_dir>/<name>.js` or
                // `<module_dir>/<name>/index.js` exists and publish it as a web component.
                let mut page_web_components: BTreeMap<(String, String), String> = BTreeMap::new();
                let mut page_registrations = String::new();
                let component_path = PathBuf::from("_notist/pages")
                    .join(&document.page.route.path)
                    .with_extension("js");
                for (module_rel, name) in &document.custom_nodes {
                    if let Some((tag, entry_out)) = resolve_ssg_web_component(
                        vault.resources(),
                        &vroot,
                        module_rel,
                        name,
                        &mut assets,
                        &mut published_component_files,
                    )? {
                        let import_rel = relative_url(&component_path, &entry_out, false);
                        let _ = std::fmt::Write::write_fmt(
                            &mut page_registrations,
                            format_args!(
                                "import * as _c_{idx} from \"./{import_rel}\";\nif (!customElements.get(\"{tag}\")) {{ const C = _c_{idx}.default ?? Object.values(_c_{idx}).find(v => typeof v === 'function'); if (C) customElements.define(\"{tag}\", C); }}\n",
                                idx = page_web_components.len(),
                            ),
                        );
                        page_web_components.insert((module_rel.clone(), name.clone()), tag);
                    }
                }
                let mut math_tag: Option<&str> = None;
                if document.has_math
                    && let Some(math_comp) = &config.math_component
                    && let Some(entry_out) = resolve_ssg_math_component(
                        vault.resources(),
                        math_comp,
                        &mut assets,
                        &mut published_component_files,
                    )?
                {
                    let import_rel = relative_url(&component_path, &entry_out, false);
                    let _ = std::fmt::Write::write_fmt(
                        &mut page_registrations,
                        format_args!(
                            "import * as _c_math from \"./{import_rel}\";\nif (!customElements.get(\"notist-math\")) {{ const C = _c_math.default ?? Object.values(_c_math).find(v => typeof v === 'function'); if (C) customElements.define(\"notist-math\", C); }}\n",
                        ),
                    );
                    math_tag = Some("notist-math");
                }
                if !page_registrations.is_empty() {
                    components.insert(
                        document.page.route.path.clone(),
                        relative_url(&document.page.route.output, &component_path, false),
                    );
                    assets.generated(component_path, page_registrations.into_bytes())?;
                }
                let html = document.render_html(&resolved, &page_web_components, math_tag);
                let mut page = document.page;
                page.html = html;
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
                files: BTreeMap::new(),
            };
            crate::extensions::before_template(
                vault.resources(),
                config,
                &mut extensions,
                &mut template_input,
                &assets,
            )?;
            for (path, bytes) in template_input.files {
                assets.generated(path, bytes)?;
            }
            let data = template_input.data;
            crate::extensions::finish(&mut extensions)?;
            let templates_stage = notist::progress::Span::stage("site.templates", || {
                format!("Render templates ({} pages)", pages.len())
            });
            let engine = templates::load(vault.resources(), config)?;
            let dirs = directories(&pages, &source_dirs, &content);
            let values = TemplateContext::new(config, &pages, &dirs, &data)?;
            let rendered = render_templates(
                &engine,
                config,
                &pages,
                &values,
                &components,
                &highlight_css,
                executor,
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
        },
    )
}

/// Resolve an SSG-only Web Component entry (`<dir>/<name>.js` or `<dir>/<name>/index.js`)
/// by convention for a custom content node declared in `module_rel`.
/// Returns `Some((custom_element_tag, published_entry_path))` if a JS implementation exists.
fn resolve_ssg_web_component<R: Resources>(
    resources: &R,
    vroot: &Path,
    module_rel: &str,
    name: &str,
    assets: &mut Assets,
    published: &mut BTreeSet<PathBuf>,
) -> Result<Option<(String, PathBuf)>, Error> {
    let module_fs = vroot.join(module_rel);
    let Some(module_dir) = module_fs.parent() else {
        return Ok(None);
    };
    let single_file = module_dir.join(format!("{name}.js"));
    let dir_entry = module_dir.join(name).join("index.js");
    let (entry_fs, copy_dir) = if resources.kind(&single_file)? == Some(ResourceKind::File) {
        (single_file, None)
    } else if resources.kind(&dir_entry)? == Some(ResourceKind::File) {
        (dir_entry, Some(module_dir.join(name)))
    } else {
        return Ok(None);
    };

    let mod_rel_dir = Path::new(module_rel).parent().unwrap_or(Path::new(""));
    let out_base = PathBuf::from("_notist/components").join(mod_rel_dir);

    let entry_out = if let Some(dir) = copy_dir {
        let dir_out = out_base.join(name);
        copy_component_dir(resources, &dir, &dir_out, assets, published)?;
        dir_out.join("index.js")
    } else {
        let file_out = out_base.join(format!("{name}.js"));
        if published.insert(file_out.clone()) {
            assets.copy(resources, &entry_fs, &file_out)?;
        }
        file_out
    };

    let tag = format!(
        "notist-{}-{}",
        sanitize_tag_part(mod_rel_dir.to_string_lossy().as_ref()),
        sanitize_tag_part(name)
    );
    Ok(Some((tag, entry_out)))
}

fn resolve_ssg_math_component<R: Resources>(
    resources: &R,
    math_component: &Path,
    assets: &mut Assets,
    published: &mut BTreeSet<PathBuf>,
) -> Result<Option<PathBuf>, Error> {
    let entry_fs = resources.resolve(math_component);
    let kind = resources.kind(&entry_fs)?;
    let out_base = PathBuf::from("_notist/components/math");
    match kind {
        Some(ResourceKind::File) => {
            if entry_fs.file_name().is_some_and(|n| n == "index.js")
                && let Some(dir) = entry_fs.parent()
            {
                copy_component_dir(resources, dir, &out_base, assets, published)?;
                Ok(Some(out_base.join("index.js")))
            } else {
                let file_name = entry_fs
                    .file_name()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("math.js"));
                let file_out = out_base.join(file_name);
                if published.insert(file_out.clone()) {
                    assets.copy(resources, &entry_fs, &file_out)?;
                }
                Ok(Some(file_out))
            }
        }
        Some(ResourceKind::Directory) => {
            let index = entry_fs.join("index.js");
            if resources.kind(&index)? == Some(ResourceKind::File) {
                copy_component_dir(resources, &entry_fs, &out_base, assets, published)?;
                Ok(Some(out_base.join("index.js")))
            } else {
                Ok(None)
            }
        }
        None => Ok(None),
    }
}

fn copy_component_dir<R: Resources>(
    resources: &R,
    src_dir: &Path,
    out_dir: &Path,
    assets: &mut Assets,
    published: &mut BTreeSet<PathBuf>,
) -> Result<(), Error> {
    for entry in resources.entries(src_dir)? {
        let Some(file_name) = entry.file_name() else {
            continue;
        };
        let target = out_dir.join(file_name);
        match resources.kind(&entry)? {
            Some(ResourceKind::Directory) => {
                copy_component_dir(resources, &entry, &target, assets, published)?;
            }
            Some(ResourceKind::File) => {
                if published.insert(target.clone()) {
                    assets.copy(resources, &entry, &target)?;
                }
            }
            None => {}
        }
    }
    Ok(())
}

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

#[allow(clippy::type_complexity)]
fn compile_notc_site<R: Resources>(
    resources: &R,
    routes: &[crate::Route],
    documents: BTreeMap<PathBuf, String>,
) -> Result<
    (
        PathBuf,
        Vec<(crate::Route, String, Arc<str>)>,
        Option<notc::CheckedProject>,
        Vec<SourceDiagnostic>,
    ),
    Error,
> {
    if routes.is_empty() {
        return Ok((resources.root().to_path_buf(), Vec::new(), None, Vec::new()));
    }

    let mut all_paths: Vec<PathBuf> = vec![resources.root().to_path_buf()];
    for route in routes {
        all_paths.push(route.source.clone());
    }
    let vroot = common_root_dir(&all_paths);

    let mut errors: Vec<SourceDiagnostic> = Vec::new();
    let mut doc_entries: Vec<(crate::Route, String, Arc<str>)> = Vec::with_capacity(routes.len());
    let mut parsed_modules: Vec<(String, PathBuf, Arc<str>, Module)> = Vec::new();
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut imports_by_file: HashMap<String, Vec<String>> = HashMap::new();

    for ((_, text), route) in documents.into_iter().zip(routes.iter().cloned()) {
        let virtual_path = to_virtual_path(&vroot, &route.source);
        let source_arc: Arc<str> = Arc::from(text.as_str());
        let parsed = if route.source.extension().is_some_and(|e| e == "md") {
            parse_markdown_document(&source_arc)
        } else {
            parse_document(&source_arc)
        };
        for d in parsed.diagnostics {
            let range = TextRange::new(d.span.start.into(), d.span.end.into());
            errors.push(diagnostic(
                &route.source,
                &source_arc,
                Diagnostic::new(Phase::Syntax, range, d.message),
            ));
        }
        let mut refs = Vec::new();
        for decl in &parsed.program.root.declarations {
            if let Decl::Import(import) = decl {
                refs.push(import.path.clone());
            }
        }
        visited.insert(virtual_path.clone());
        imports_by_file.insert(virtual_path.clone(), refs);
        queue.push_back(virtual_path.clone());
        doc_entries.push((route.clone(), virtual_path.clone(), Arc::clone(&source_arc)));
        parsed_modules.push((virtual_path, route.source, source_arc, parsed.program.root));
    }

    // Transitively load all `.notc` modules imported by documents or other modules
    // through `resources` so watch/serve automatically observes them.
    while let Some(from_rel) = queue.pop_front() {
        let Some(refs) = imports_by_file.remove(&from_rel) else {
            continue;
        };
        for import_path in refs {
            let Ok(target_rel) = resolve_import_path(&from_rel, &import_path) else {
                continue;
            };
            if !visited.insert(target_rel.clone()) {
                continue;
            }
            let target_fs = vroot.join(&target_rel);
            let Ok(bytes) = resources.read(&target_fs) else {
                // Let `check_parsed_modules` report `MissingModule` at the exact import span.
                continue;
            };
            let Ok(text) = String::from_utf8(bytes) else {
                continue;
            };
            let source_arc: Arc<str> = Arc::from(text.as_str());
            let parsed = parse(&source_arc);
            for d in parsed.diagnostics {
                let range = TextRange::new(d.span.start.into(), d.span.end.into());
                errors.push(diagnostic(
                    &target_fs,
                    &source_arc,
                    Diagnostic::new(Phase::Syntax, range, d.message),
                ));
            }
            let mut next_refs = Vec::new();
            for decl in &parsed.program.root.declarations {
                if let Decl::Import(import) = decl {
                    next_refs.push(import.path.clone());
                }
            }
            imports_by_file.insert(target_rel.clone(), next_refs);
            queue.push_back(target_rel.clone());
            parsed_modules.push((target_rel, target_fs, source_arc, parsed.program.root));
        }
    }

    let meta_by_rel: HashMap<&str, (&Path, &str)> = parsed_modules
        .iter()
        .map(|(rel, fs, src, _)| (rel.as_str(), (fs.as_path(), src.as_ref())))
        .collect();
    let module_refs: Vec<(&str, &Module)> = parsed_modules
        .iter()
        .map(|(rel, _, _, module)| (rel.as_str(), module))
        .collect();

    let checked = match check_parsed_modules(&module_refs) {
        Ok(project) => Some(project),
        Err(project_diags) => {
            for diag in project_diags {
                let (fs_path, src) = meta_by_rel
                    .get(diag.module.as_str())
                    .copied()
                    .unwrap_or((Path::new(&diag.module), ""));
                let (phase, span, msg) = match diag.kind {
                    ProjectDiagnosticKind::Parse(p) => (Phase::Syntax, p.span, p.message),
                    ProjectDiagnosticKind::Type { span, error, .. } => {
                        (Phase::Type, span, format_type_error(&error))
                    }
                };
                let range = TextRange::new(span.start.into(), span.end.into());
                errors.push(diagnostic(fs_path, src, Diagnostic::new(phase, range, msg)));
            }
            None
        }
    };

    Ok((vroot, doc_entries, checked, errors))
}

fn common_root_dir(paths: &[PathBuf]) -> PathBuf {
    let dirs: Vec<PathBuf> = paths
        .iter()
        .map(|p| {
            if p.extension().is_some() {
                p.parent().unwrap_or(p).to_path_buf()
            } else {
                p.clone()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Route;
    use crate::document::Document;

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
            let root = notc::Block::empty(notc::Span::DUMMY)
                .with_attr(Arc::from("id"), notc::AttrValue::String(Arc::from(id)));
            Document::from_block(
                Route::new(content.join(source), content).unwrap(),
                root,
                "Site",
            )
            .unwrap()
        })
        .collect();
        let index = DocumentIndex::new(documents.iter().map(|d| (&d.page.route, &d.ids)));
        documents.reverse();
        let reversed = DocumentIndex::new(documents.iter().map(|d| (&d.page.route, &d.ids)));
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
        let serial_executor = notist::Executor::new(std::num::NonZeroUsize::MIN).unwrap();
        let parallel_executor =
            notist::Executor::new(std::num::NonZeroUsize::new(4).unwrap()).unwrap();
        let content = Path::new("/site/content");
        let pages: Vec<_> = (0..521)
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
        let serial = render_templates(
            &engine,
            &config,
            &pages,
            &values,
            &components,
            highlight,
            &serial_executor,
        )
        .unwrap();
        let (parallel, timings) = notist::timings::capture(|| {
            let parent = notist::progress::Span::stage("site.templates", || "Templates".into());
            let result = render_templates(
                &engine,
                &config,
                &pages,
                &values,
                &components,
                highlight,
                &parallel_executor,
            )
            .unwrap();
            parent.finish();
            result
        });
        assert_eq!(timings.spans.len(), 522);
        assert!(
            timings.spans[1..]
                .iter()
                .all(|span| span.parent == Some(0) && span.status == "finished")
        );
        assert!(
            timings.spans[1..]
                .iter()
                .map(|span| span.lane)
                .collect::<BTreeSet<_>>()
                .len()
                <= parallel_executor.jobs()
        );
        assert_eq!(serial, parallel);
        assert!(
            String::from_utf8(parallel[7].clone())
                .unwrap()
                .starts_with("page-7|<strong>7</strong>|521|521")
        );
        let mut invalid = pages;
        for index in [3, 318] {
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
            &serial_executor,
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
            &parallel_executor,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(serial, parallel);
        assert!(serial.contains("p3.not"));
    }
}
