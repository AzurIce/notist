//! Chapter discovery, navigation and HTML book rendering over a Vault.
//! the result is an owned publication artifact, not a filesystem mutation.
use notist::resources::{ResourceError, ResourceKind};
use notist::{
    Ctor, Diagnostic, DiagnosticCode, EvaluationState, Item, Phase, Resources, SourceDiagnostic,
    TextRange, Value, Vault,
};
use notist_html::Renderer;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

mod config;
mod embeds;
mod layout;
mod mounts;
mod outline;
mod routes;
mod structure;
mod summary;
mod theme;
pub use config::{BookConfig, Mount};
use layout::Layout;
pub use layout::{Navigation, PageLink};
use mounts::Mounts;
use notist::publication::{self, Assets};
pub use outline::Heading;
use outline::Outline;
pub use routes::{Route, Routes};
pub use structure::NavigationKind;
use theme::Theme;

#[derive(Debug)]
pub enum Error {
    Diagnostics(Vec<SourceDiagnostic>),
    Message(String),
    Resource(ResourceError),
    Template(String),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Diagnostics(errors) => {
                for error in errors {
                    writeln!(f, "{}: {}", error.path.display(), error.diagnostic.message)?;
                }
                Ok(())
            }
            Self::Message(message) => f.write_str(message),
            Self::Resource(error) => error.fmt(f),
            Self::Template(error) => write!(f, "{error:#}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<ResourceError> for Error {
    fn from(error: ResourceError) -> Self {
        Self::Resource(error)
    }
}
impl From<handlebars::TemplateError> for Error {
    fn from(error: handlebars::TemplateError) -> Self {
        Self::Template(error.to_string())
    }
}
impl From<handlebars::RenderError> for Error {
    fn from(error: handlebars::RenderError) -> Self {
        Self::Template(error.to_string())
    }
}
impl From<notist::VaultError> for Error {
    fn from(error: notist::VaultError) -> Self {
        match error {
            notist::VaultError::Environment(errors) => Self::Diagnostics(errors),
            notist::VaultError::Resource(error) => Self::Resource(error),
            other => Self::Message(other.to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub route: Route,
    pub title: String,
    pub headings: Vec<Heading>,
}
/// Fully validated files. A failed build returns no partial publication.
#[derive(Debug)]
pub struct BuildOutput {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub pages: Vec<Page>,
    /// Content failures recovered during serve.
    pub diagnostics: Vec<SourceDiagnostic>,
    /// Broken hyperlinks that do not prevent publication.
    pub warnings: Vec<SourceDiagnostic>,
}
struct Prepared {
    route: Route,
    source: std::sync::Arc<str>,
    root: Item,
    outline: Outline,
    sources: notist::Sources,
}

fn content_diagnostic(
    page: &Prepared,
    node: &Item,
    message: impl Into<String>,
) -> SourceDiagnostic {
    let span = node.origin.as_ref().map_or(node.span, |origin| {
        TextRange::new(origin.start.into(), origin.end.into())
    });
    let mut diagnostic = Diagnostic::new(Phase::Semantic, span, message);
    diagnostic.source = node.origin.as_ref().map(|origin| origin.source.clone());
    locate(&page.route.source, &page.source, diagnostic, &page.sources)
}

fn locate(
    path: &Path,
    source: &str,
    diagnostic: Diagnostic,
    sources: &notist::Sources,
) -> SourceDiagnostic {
    let origin = diagnostic
        .source
        .as_ref()
        .and_then(|s| sources.get_key_value(s));
    SourceDiagnostic {
        path: origin
            .map(|(p, _)| PathBuf::from(p))
            .unwrap_or_else(|| path.into()),
        source: origin
            .map(|(_, s)| s.to_string())
            .unwrap_or_else(|| source.into()),
        diagnostic,
    }
}

fn append<'a>(
    errors: &mut Vec<SourceDiagnostic>,
    path: &Path,
    source: &str,
    values: impl IntoIterator<Item = &'a Diagnostic>,
    sources: &notist::Sources,
) {
    errors.extend(
        values
            .into_iter()
            .cloned()
            .map(|d| locate(path, source, d, sources)),
    );
}

fn discover<R: Resources>(
    vault: &Vault<R>,
    config: &BookConfig,
    mounts: &Mounts,
) -> Result<BTreeSet<PathBuf>, Error> {
    let globs = |patterns: &[String]| {
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(
                globset::GlobBuilder::new(pattern)
                    .literal_separator(true)
                    .build()
                    .map_err(|error| {
                        Error::Message(format!("invalid book glob `{pattern}`: {error}"))
                    })?,
            );
        }
        builder
            .build()
            .map_err(|error| Error::Message(error.to_string()))
    };
    let include = globs(&config.include)?;
    let exclude = globs(&config.exclude)?;
    let resources = vault.resources();
    let output = resources.resolve(&config.output);
    if output == resources.root() {
        return Err(Error::Message(
            "site output cannot replace the Vault root".into(),
        ));
    }
    let theme = config
        .theme
        .as_ref()
        .map(|path| mounts.resolve(&resources.resolve(path)));
    let mut pending: Vec<_> = mounts
        .roots()
        .map(|(source, _)| source.to_path_buf())
        .collect();
    let mut pages = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        if directory.starts_with(&output)
            || theme
                .as_ref()
                .is_some_and(|theme| directory.starts_with(theme))
        {
            continue;
        }
        let relative = mounts.path(&directory).unwrap();
        if relative
            .ancestors()
            .filter(|path| !path.as_os_str().is_empty())
            .any(|path| {
                exclude.is_match(path)
                    || path.file_name().is_some_and(|name| {
                        name.to_string_lossy().starts_with('.')
                            || name == "target"
                            || name == "node_modules"
                    })
            })
        {
            continue;
        }
        for entry in resources.entries(&directory)? {
            if entry.starts_with(&output)
                || theme.as_ref().is_some_and(|theme| entry.starts_with(theme))
            {
                continue;
            }
            let relative = mounts
                .path(&entry)
                .ok_or_else(|| Error::Message("discovery escaped book sources".into()))?;
            if exclude.is_match(&relative)
                || entry.file_name().is_some_and(|name| {
                    name.to_string_lossy().starts_with('.')
                        || name == "target"
                        || name == "node_modules"
                })
            {
                continue;
            }
            match resources.kind(&entry)? {
                Some(ResourceKind::Directory) => pending.push(entry),
                Some(ResourceKind::File)
                    if include.is_match(&relative) && vault.supports(&entry) =>
                {
                    pages.insert(entry);
                }
                _ => {}
            }
        }
    }
    Ok(pages)
}

/// Analyze each selected document once, validate routes/references, then render
/// transformed trees with one shared outline and resource publication plan.
pub fn build<R: Resources>(
    vault: &mut Vault<R>,
    config: &BookConfig,
) -> Result<BuildOutput, Error> {
    build_inner(vault, config, false)
}

/// Render recoverable Code failures as error nodes while retaining strict
/// routing, resource, configuration and publication checks.
pub fn serve<R: Resources>(
    vault: &mut Vault<R>,
    config: &BookConfig,
) -> Result<BuildOutput, Error> {
    build_inner(vault, config, true)
}

fn build_inner<R: Resources>(
    vault: &mut Vault<R>,
    config: &BookConfig,
    recover: bool,
) -> Result<BuildOutput, Error> {
    let mounts = Mounts::new(vault.resources(), config)?;
    let mut sources = discover(vault, config, &mounts)?;
    let root = vault.resources().root().to_path_buf();
    let mut summary_path = None;
    for name in summary::FILE_NAMES {
        let path = root.join(name);
        sources.remove(&path);
        if vault.resources().kind(&path)? == Some(ResourceKind::File) {
            if summary_path.is_some() {
                return Err(Error::Message(
                    "both SUMMARY.not and SUMMARY.md exist; keep one book structure file".into(),
                ));
            }
            summary_path = Some(path);
        }
    }
    let mut errors = vec![];
    let summary = if let Some(summary_path) = summary_path {
        let source = vault.resources().source(&summary_path)?;
        let analysis = vault.analyze(&summary_path, &source)?;
        append(
            &mut errors,
            &summary_path,
            &source,
            analysis
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code != Some(DiagnosticCode::OutsideVault)),
            analysis.sources(),
        );
        let entries = summary::interpret(analysis.root()).and_then(|entries| {
            summary::resolve(
                &entries,
                &summary_path,
                &sources,
                vault.resources(),
                &mounts,
            )
        });
        match entries {
            Ok(entries) => Some(entries),
            Err(diagnostics) => {
                append(
                    &mut errors,
                    &summary_path,
                    &source,
                    &diagnostics,
                    analysis.sources(),
                );
                None
            }
        }
    } else {
        None
    };
    if !errors.is_empty() {
        return Err(Error::Diagnostics(errors));
    }
    if let Some(plan) = &summary {
        sources.retain(|source| plan.includes(source, &mounts));
    }
    if sources.is_empty() {
        return Err(Error::Message("book contains no selected documents".into()));
    }
    let routes = sources
        .into_iter()
        .map(|source| {
            let path = mounts.path(&source).unwrap();
            Route::new(source, path)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(Error::Message)?;
    let routes = Routes::new(routes).map_err(Error::Message)?;
    let summary = summary.map(|plan| plan.expand(&routes));
    let mut recovered = vec![];
    let mut prepared = vec![];
    for route in routes.iter() {
        let source = vault.resources().source(&route.source)?;
        let analysis = vault.analyze(&route.source, &source)?;
        // Publication checks the evaluated references below. Defer the Vault's
        // source-level boundary checks so hyperlinks can warn and retain URLs.
        append(
            &mut errors,
            &route.source,
            &source,
            analysis
                .diagnostics()
                .iter()
                .filter(|diagnostic| diagnostic.code != Some(DiagnosticCode::OutsideVault)),
            analysis.sources(),
        );
        let mut index_errors = vec![];
        let _index = notist::index::Index::build(analysis.root(), &mut index_errors);
        append(
            &mut errors,
            &route.source,
            &source,
            &index_errors,
            analysis.sources(),
        );
        let evaluation = vault.evaluate(&route.source, &analysis)?;
        append(
            &mut errors,
            &route.source,
            &source,
            &evaluation.diagnostics,
            analysis.sources(),
        );
        let mut content = evaluation.root;
        let fallback = if route.url == "/" {
            config.title.as_str()
        } else {
            route.source.file_stem().unwrap().to_str().unwrap_or("Page")
        };
        let outline = outline::prepare(&mut content, fallback);
        prepared.push(Prepared {
            route: route.clone(),
            source: analysis.sources()[&route.source.to_string_lossy().into_owned()].clone(),
            root: content,
            outline,
            sources: analysis.sources().clone(),
        });
    }
    if recover {
        let (local, fatal): (Vec<_>, Vec<_>) =
            errors.into_iter().partition(|e| e.diagnostic.recovered);
        recovered = local;
        errors = fatal;
    }
    if !errors.is_empty() {
        return Err(Error::Diagnostics(errors));
    }
    let pages: Vec<_> = prepared
        .iter()
        .map(|page| Page {
            route: page.route.clone(),
            title: page.outline.title.clone(),
            headings: page.outline.headings.clone(),
        })
        .collect();
    let ids: BTreeMap<_, _> = prepared
        .iter()
        .map(|page| (page.route.source.clone(), &page.outline.ids))
        .collect();
    let mut assets = Assets::new(vault.resources().resolve(&config.output));
    let mut warnings = vec![];
    let theme = Theme::load(vault.resources(), config, &mounts, &mut assets)?;
    let layout = match &summary {
        Some(entries) => Layout::declared(entries, &pages),
        None => Layout::new(&config.title, &pages),
    };
    for page in &prepared {
        let mut resolved = BTreeMap::new();
        for node in page.root.descendants().filter(|node| {
            matches!(node.ctor, Ctor::Link | Ctor::Embed)
                && !matches!(node.evaluation, Some(EvaluationState::Error(_)))
        }) {
            let Some(Value::Str(value)) = node.fields.get("target") else {
                continue;
            };
            match routes::target(&page.route.source, value) {
                Ok(routes::Target::External) => {}
                Err(message) => errors.push(content_diagnostic(page, node, message)),
                Ok(routes::Target::Local {
                    path,
                    query,
                    fragment,
                }) => {
                    let path = mounts.resolve(&path);
                    let Some(book_path) = mounts.path(&path) else {
                        let mut diagnostic = content_diagnostic(
                            page,
                            node,
                            format!(
                                "local content reference `{value}` is outside Vault root `{}` and configured book mounts",
                                root.display()
                            ),
                        );
                        diagnostic.diagnostic.code = Some(DiagnosticCode::OutsideVault);
                        if node.ctor == Ctor::Link {
                            warnings.push(diagnostic);
                        } else {
                            errors.push(diagnostic);
                        }
                        continue;
                    };
                    let mut url = if let Some(route) = routes.get(&path) {
                        if let Some(anchor) = &fragment
                            && !anchor.is_empty()
                            && !ids[&path].contains(anchor)
                        {
                            let diagnostics = if node.ctor == Ctor::Link {
                                &mut warnings
                            } else {
                                &mut errors
                            };
                            diagnostics.push(content_diagnostic(
                                page,
                                node,
                                format!("missing anchor `#{anchor}` in `{}`", path.display()),
                            ));
                        }
                        if path == page.route.source {
                            String::new()
                        } else {
                            notist::publication::url::relative_url(
                                &page.route.output,
                                route.output.parent().unwrap(),
                                true,
                            )
                        }
                    } else if vault.supports(&path) {
                        let diagnostic = content_diagnostic(
                            page,
                            node,
                            format!(
                                "document `{}` is not published by this site",
                                path.display()
                            ),
                        );
                        if node.ctor != Ctor::Link {
                            errors.push(diagnostic);
                            continue;
                        }
                        warnings.push(diagnostic);
                        let route = match Route::new(path.clone(), book_path) {
                            Ok(route) => route,
                            Err(message) => {
                                errors.push(content_diagnostic(page, node, message));
                                continue;
                            }
                        };
                        notist::publication::url::relative_url(
                            &page.route.output,
                            route.output.parent().unwrap(),
                            true,
                        )
                    } else {
                        let destination = book_path.as_path();
                        if destination.starts_with("_notist") {
                            errors.push(content_diagnostic(
                                page,
                                node,
                                "_notist is reserved for generated resources",
                            ));
                            continue;
                        }
                        if vault.resources().kind(&path)? != Some(ResourceKind::File) {
                            let diagnostic = content_diagnostic(
                                page,
                                node,
                                format!("missing local resource `{value}`"),
                            );
                            if node.ctor != Ctor::Link {
                                errors.push(diagnostic);
                                continue;
                            }
                            warnings.push(diagnostic);
                        } else {
                            assets.copy(vault.resources(), &path, destination)?;
                        }
                        notist::publication::url::relative_url(
                            &page.route.output,
                            destination,
                            false,
                        )
                    };
                    url.push_str(&query);
                    if let Some(anchor) = fragment {
                        url.push('#');
                        url.push_str(&notist::publication::url::fragment(&anchor));
                    }
                    if url.is_empty() {
                        url.push_str("./");
                    }
                    resolved.insert(value.clone(), url);
                }
            }
        }
        let registry = vault.html_registry(&page.route.source, &page.source, &page.root)?;
        let rendered = Renderer::new()
            .with_registry(registry)
            .with_url_resolver(|_, url| resolved.get(url).cloned())
            .with_embed_renderer(|item| embeds::image(item, &resolved))
            .render_with_diagnostics(&page.root);
        append(
            &mut errors,
            &page.route.source,
            &page.source,
            &rendered.diagnostics,
            &page.sources,
        );
        let environment = vault.environment_for(&page.route.source, &page.source)?;
        let component_file = page.route.output.parent().unwrap().join("components.js");
        let registrations = assets.components(
            vault.resources(),
            &environment,
            &rendered.used_components,
            &component_file,
        )?;
        let component_url = if registrations.is_empty() {
            None
        } else {
            assets.generated(component_file, registrations.into_bytes())?;
            Some("./components.js")
        };
        let (previous, next) = layout.adjacent(&page.route.output, &pages);
        let context = serde_json::json!({
            "book":{"title":config.title,"home":layout.home_url(&page.route.output)},
            "page":{"title":page.outline.title,"url":page.route.url,"source":page.route.path,"html":rendered.html},
            "navigation":layout.navigation(&page.route.output),"breadcrumbs":layout.breadcrumbs(&page.route.output),
            "outline":page.outline.headings.iter().map(|heading| serde_json::json!({"title":heading.title,"id":heading.id,"anchor":notist::publication::url::fragment(&heading.id),"level":heading.level})).collect::<Vec<_>>(),
            "previous":previous,"next":next,
            "assets":{"styles":theme.styles(&page.route.output),"scripts":theme.scripts(&page.route.output),"components":component_url}
        });
        assets.generated(
            page.route.output.clone(),
            theme.render(context)?.into_bytes(),
        )?;
    }
    if !errors.is_empty() {
        return Err(Error::Diagnostics(errors));
    }
    let mut site = finish(assets, layout.ordered_pages(pages))?;
    site.diagnostics = recovered;
    site.warnings = warnings;
    Ok(site)
}

fn finish(assets: Assets, pages: Vec<Page>) -> Result<BuildOutput, Error> {
    Ok(BuildOutput {
        files: assets.finish()?,
        pages,
        diagnostics: Vec::new(),
        warnings: Vec::new(),
    })
}

impl From<publication::Error> for Error {
    fn from(error: publication::Error) -> Self {
        match error {
            publication::Error::Message(message) => Self::Message(message),
            publication::Error::Resource(error) => Self::Resource(error),
        }
    }
}
