//! Vault-scoped static site planning and rendering. IO is supplied by Resources;
//! the result is an owned publication artifact, not a filesystem mutation.
use notist::resources::{ResourceError, ResourceKind};
use notist::{Ctor, Diagnostic, Item, Phase, Resources, SourceDiagnostic, TextRange, Value, Vault};
use notist_html::Renderer;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

mod assets;
mod config;
mod layout;
mod outline;
mod routes;
mod theme;
use assets::Assets;
pub use assets::file_url;
pub use config::SiteConfig;
use layout::Layout;
pub use layout::{Navigation, PageLink};
pub use outline::Heading;
use outline::Outline;
pub use routes::{Route, Routes};
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
pub struct Site {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub pages: Vec<Page>,
}
struct Prepared {
    route: Route,
    source: String,
    root: Item,
    outline: Outline,
}

fn diagnostic(
    path: &Path,
    source: &str,
    span: TextRange,
    message: impl Into<String>,
) -> SourceDiagnostic {
    SourceDiagnostic {
        path: path.into(),
        source: source.into(),
        diagnostic: Diagnostic::new(Phase::Semantic, span, message),
    }
}
fn append(errors: &mut Vec<SourceDiagnostic>, path: &Path, source: &str, values: &[Diagnostic]) {
    errors.extend(values.iter().cloned().map(|value| SourceDiagnostic {
        path: path.into(),
        source: source.into(),
        diagnostic: value,
    }));
}

fn discover<R: Resources>(vault: &Vault<R>, config: &SiteConfig) -> Result<Vec<PathBuf>, Error> {
    let globs = |patterns: &[String]| {
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(
                globset::GlobBuilder::new(pattern)
                    .literal_separator(true)
                    .build()
                    .map_err(|error| {
                        Error::Message(format!("invalid site glob `{pattern}`: {error}"))
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
    let theme = config.theme.as_ref().map(|path| resources.resolve(path));
    let mut pending = vec![resources.root().to_path_buf()];
    let mut pages = vec![];
    while let Some(directory) = pending.pop() {
        for entry in resources.entries(&directory)? {
            if entry.starts_with(&output)
                || theme.as_ref().is_some_and(|theme| entry.starts_with(theme))
            {
                continue;
            }
            let relative = entry
                .strip_prefix(resources.root())
                .map_err(|_| Error::Message("discovery escaped the Vault".into()))?;
            if exclude.is_match(relative)
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
                    if include.is_match(relative) && vault.supports(&entry) =>
                {
                    pages.push(entry)
                }
                _ => {}
            }
        }
    }
    pages.sort();
    Ok(pages)
}

/// Analyze each selected document once, validate routes/references, then render
/// transformed trees with one shared outline and resource publication plan.
pub fn build<R: Resources>(vault: &mut Vault<R>, config: &SiteConfig) -> Result<Site, Error> {
    let sources = discover(vault, config)?;
    if sources.is_empty() {
        return Err(Error::Message("site contains no selected documents".into()));
    }
    let root = vault.resources().root().to_path_buf();
    let routes = Routes::new(&root, sources).map_err(Error::Message)?;
    let mut errors = vec![];
    let mut prepared = vec![];
    for route in routes.iter() {
        let source = vault.resources().source(&route.source)?;
        let analysis = vault.analyze(&route.source, &source)?;
        append(&mut errors, &route.source, &source, analysis.diagnostics());
        let mut index_errors = vec![];
        let _index = notist::index::Index::build(analysis.root(), &mut index_errors);
        append(&mut errors, &route.source, &source, &index_errors);
        let transformed = vault.transform(&route.source, analysis.root())?;
        append(
            &mut errors,
            &route.source,
            &source,
            &transformed.diagnostics,
        );
        let mut content = transformed.root;
        let fallback = if route.url == "/" {
            config.title.as_str()
        } else {
            route.source.file_stem().unwrap().to_str().unwrap_or("Page")
        };
        let outline = outline::prepare(&mut content, fallback);
        prepared.push(Prepared {
            route: route.clone(),
            source,
            root: content,
            outline,
        });
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
    let theme = Theme::load(vault.resources(), config, &mut assets)?;
    let layout = Layout::new(&config.title, &pages);
    for page in &prepared {
        let mut resolved = BTreeMap::new();
        for node in page
            .root
            .descendants()
            .filter(|node| matches!(node.ctor, Ctor::Link | Ctor::Embed))
        {
            let Some(Value::Str(value)) = node.fields.get("target") else {
                continue;
            };
            match routes::target(&page.route.source, value) {
                Ok(routes::Target::External) => {}
                Err(message) => errors.push(diagnostic(
                    &page.route.source,
                    &page.source,
                    node.span,
                    message,
                )),
                Ok(routes::Target::Local {
                    path,
                    query,
                    fragment,
                }) => {
                    if !path.starts_with(&root) {
                        errors.push(diagnostic(
                            &page.route.source,
                            &page.source,
                            node.span,
                            format!(
                                "local content reference `{value}` is outside Vault root `{}`",
                                root.display()
                            ),
                        ));
                        continue;
                    }
                    let mut url = if let Some(route) = routes.get(&path) {
                        if let Some(anchor) = &fragment
                            && !anchor.is_empty()
                            && !ids[&path].contains(anchor)
                        {
                            errors.push(diagnostic(
                                &page.route.source,
                                &page.source,
                                node.span,
                                format!("missing anchor `#{anchor}` in `{}`", path.display()),
                            ));
                        }
                        if path == page.route.source {
                            String::new()
                        } else {
                            routes::relative_url(
                                &page.route.output,
                                route.output.parent().unwrap(),
                                true,
                            )
                        }
                    } else if vault.supports(&path) {
                        errors.push(diagnostic(
                            &page.route.source,
                            &page.source,
                            node.span,
                            format!(
                                "document `{}` is not published by this site",
                                path.display()
                            ),
                        ));
                        continue;
                    } else {
                        if vault.resources().kind(&path)? != Some(ResourceKind::File) {
                            errors.push(diagnostic(
                                &page.route.source,
                                &page.source,
                                node.span,
                                format!("missing local resource `{value}`"),
                            ));
                            continue;
                        }
                        let destination = path.strip_prefix(&root).unwrap();
                        if destination.starts_with("_notist") {
                            errors.push(diagnostic(
                                &page.route.source,
                                &page.source,
                                node.span,
                                "_notist is reserved for generated resources",
                            ));
                            continue;
                        }
                        assets.copy(vault.resources(), &path, destination)?;
                        routes::relative_url(&page.route.output, destination, false)
                    };
                    url.push_str(&query);
                    if let Some(anchor) = fragment {
                        url.push('#');
                        url.push_str(&routes::fragment(&anchor));
                    }
                    if url.is_empty() {
                        url.push_str("./");
                    }
                    resolved.insert(value.clone(), url);
                }
            }
        }
        let registry = vault.html_registry(&page.route.source)?;
        let rendered = Renderer::new()
            .with_registry(registry)
            .with_url_resolver(|_, url| resolved.get(url).cloned())
            .render_with_diagnostics(&page.root);
        append(
            &mut errors,
            &page.route.source,
            &page.source,
            &rendered.diagnostics,
        );
        let environment = vault.environment_for(&page.route.source)?.clone();
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
            "site":{"title":config.title,"home":layout.home_url(&page.route.output)},
            "page":{"title":page.outline.title,"url":page.route.url,"source":page.route.source.strip_prefix(&root).unwrap(),"html":rendered.html},
            "navigation":layout.navigation(&page.route.output),"breadcrumbs":layout.breadcrumbs(&page.route.output),
            "outline":page.outline.headings.iter().map(|heading| serde_json::json!({"title":heading.title,"id":heading.id,"anchor":routes::fragment(&heading.id),"level":heading.level})).collect::<Vec<_>>(),
            "previous":previous,"next":next,
            "assets":{"styles":theme.styles(&page.route.output),"script":theme.script(&page.route.output),"components":component_url}
        });
        assets.generated(
            page.route.output.clone(),
            theme.render(context)?.into_bytes(),
        )?;
    }
    if !errors.is_empty() {
        return Err(Error::Diagnostics(errors));
    }
    finish(assets, layout.ordered_pages(pages))
}

/// Publish a previously rendered single document through the same resource
/// and theme machinery. Site route/link validation belongs to `build`.
pub fn page<R: Resources>(
    rendered: notist_html::RenderResult,
    environment: &notist::Environment,
    resources: &R,
    config: &SiteConfig,
) -> Result<Site, Error> {
    if !rendered.diagnostics.is_empty() {
        return Err(Error::Message(
            rendered
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    let mut assets = Assets::new(resources.resolve(&config.output));
    let theme = Theme::load(resources, config, &mut assets)?;
    let output = Path::new("index.html");
    let registrations = assets.components(
        resources,
        environment,
        &rendered.used_components,
        Path::new("components.js"),
    )?;
    let component_url = if registrations.is_empty() {
        None
    } else {
        assets.generated("components.js".into(), registrations.into_bytes())?;
        Some("./components.js")
    };
    let context = serde_json::json!({"site":{"title":config.title,"home":"./"},"page":{"title":config.title,"url":"/","source":null,"html":rendered.html},"navigation":{"title":config.title,"url":"./","current":true,"children":[]},"breadcrumbs":[],"outline":[],"previous":null,"next":null,"assets":{"styles":theme.styles(output),"script":theme.script(output),"components":component_url}});
    assets.generated(output.into(), theme.render(context)?.into_bytes())?;
    finish(assets, vec![])
}

fn finish(mut assets: Assets, pages: Vec<Page>) -> Result<Site, Error> {
    let manifest = serde_json::json!({"format":"notist-site", "version":1, "pages":pages.iter().map(|page| &page.route.url).collect::<Vec<_>>()});
    assets.generated(
        "_notist/site.json".into(),
        serde_json::to_vec(&manifest).unwrap(),
    )?;
    Ok(Site {
        files: assets.files,
        pages,
    })
}
