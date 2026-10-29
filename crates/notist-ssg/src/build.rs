use crate::{
    Error, Page, Route, SiteConfig, assets,
    document::{Document, diagnostic},
    templates,
};
use notist::{
    Ctor, Diagnostic, DiagnosticCode, Item, Phase, Resources, SourceDiagnostic, Value, Vault,
    publication::{
        Assets,
        url::{fragment, relative_url},
    },
    resources::{ResourceKind, normalize},
};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// Owned, validated output. Publishing it is a separate host operation.
#[derive(Debug)]
pub struct BuildOutput {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub pages: Vec<Page>,
    pub warnings: Vec<SourceDiagnostic>,
}

#[derive(Serialize)]
struct Directory {
    path: String,
    name: String,
    index: Option<Page>,
    pages: Vec<Page>,
    directories: Vec<String>,
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
    for page in pages {
        let dir = result.get_mut(&page.directory).unwrap();
        if page
            .route
            .output
            .file_name()
            .is_some_and(|s| s == "index.html")
        {
            dir.index = Some(page.clone());
        } else {
            dir.pages.push(page.clone());
        }
    }
    result
}

fn discover<R: Resources>(
    vault: &Vault<R>,
    path: &Path,
    sources: &mut Vec<PathBuf>,
    assets: &mut Vec<PathBuf>,
    directories: &mut Vec<PathBuf>,
) -> Result<(), Error> {
    directories.push(path.into());
    for entry in vault.resources().entries(path)? {
        if entry
            .file_name()
            .is_some_and(|s| s.to_string_lossy().starts_with('.') || s == "Notist.toml")
        {
            continue;
        }
        match vault.resources().kind(&entry)? {
            Some(ResourceKind::Directory) => discover(vault, &entry, sources, assets, directories)?,
            Some(ResourceKind::File) if vault.supports(&entry) => sources.push(entry),
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
    d.source = node.origin.as_ref().map(|o| o.source.clone());
    diagnostic(
        &document.page.route.source,
        &document.source,
        &document.sources,
        d,
    )
}

fn resolve_link(
    document: &Document,
    target: &str,
    node: &Item,
    content: &Path,
    documents: &BTreeMap<PathBuf, &Document>,
    outputs: &BTreeSet<PathBuf>,
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
        .map(|o| Path::new(&o.source))
        .unwrap_or(&document.page.route.source);
    let source = if path.is_empty() {
        origin.into()
    } else if path.starts_with('/') {
        normalize(&content.join(path.trim_start_matches('/')))
    } else {
        normalize(&origin.parent().unwrap().join(path.as_ref()))
    };
    let published = source.strip_prefix(content).ok().map(Path::to_path_buf);
    let destination_doc = documents.get(&source).copied().or_else(|| {
        published.as_ref().and_then(|p| {
            documents
                .values()
                .find(|d| d.page.route.output == *p || d.page.route.output == p.join("index.html"))
                .copied()
        })
    });
    let destination = if let Some(destination) = destination_doc {
        if let Some(anchor) = anchor.filter(|a| !a.is_empty()) {
            let decoded = percent_encoding::percent_decode_str(anchor)
                .decode_utf8()
                .map_err(|_| "invalid UTF-8 in anchor")?;
            if !destination.ids.contains(decoded.as_ref()) {
                return Err(format!(
                    "missing anchor `{anchor}` in `{}`",
                    destination.page.route.path.display()
                ));
            }
        }
        destination.page.route.output.clone()
    } else {
        let published = published
            .ok_or_else(|| format!("reference `{target}` is outside the content directory"))?;
        if outputs.contains(&published) {
            published
        } else if outputs.contains(&published.join("index.html")) {
            published.join("index.html")
        } else {
            return Err(format!("missing local reference `{target}`"));
        }
    };
    let mut url = if path.is_empty() && destination == document.page.route.output {
        String::new()
    } else {
        relative_url(&document.page.route.output, &destination, false)
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
    let alt = match node.fields.get("description") {
        Some(Value::Str(s)) => s.as_str(),
        _ => "",
    };
    Some(format!(
        "<img src=\"{}\" alt=\"{}\">",
        notist_html::escape_attribute(url),
        notist_html::escape_attribute(alt)
    ))
}

pub(crate) fn build<R: Resources>(
    vault: &mut Vault<R>,
    config: &SiteConfig,
    extensions: &[Box<dyn crate::extensions::SiteExtension>],
) -> Result<BuildOutput, Error> {
    config.validate()?;
    let content = vault.resources().resolve(&config.content);
    let output = vault.resources().resolve(&config.output);
    if output.starts_with(&content)
        || content.starts_with(&output)
        || vault.resources().root().starts_with(&output)
    {
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
        inputs.push(theme.clone());
    }
    for input in inputs {
        let input = vault.resources().resolve(&input);
        if input.starts_with(&output)
            || output.starts_with(&input)
            || input.starts_with(&content)
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
    let mut content_assets = vec![];
    let mut source_dirs = vec![];
    if generated_only {
        source_dirs.push(content.clone());
    } else {
        discover(
            vault,
            &content,
            &mut sources,
            &mut content_assets,
            &mut source_dirs,
        )?;
    }
    let mut input = crate::extensions::SiteInput {
        documents: sources
            .into_iter()
            .map(|source| {
                Ok((
                    source.strip_prefix(&content).unwrap().to_path_buf(),
                    vault.resources().source(&source)?,
                ))
            })
            .collect::<Result<_, Error>>()?,
        data: assets::data(vault.resources(), &vault.resources().resolve(&config.data))?,
    };
    crate::extensions::prepare(vault, config, extensions, &mut input)?;
    let mut source_dirs: BTreeSet<_> = source_dirs.into_iter().collect();
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
    let mut outputs = BTreeSet::new();
    for (path, text) in input.documents {
        let route = Route::new(content.join(path), &content)?;
        if !outputs.insert(route.output.clone()) {
            return Err(Error::Message(format!(
                "route collision at `{}`",
                route.output.display()
            )));
        }
        let analysis = vault.analyze(&route.source, &text)?;
        errors.extend(
            analysis
                .diagnostics()
                .iter()
                .filter(|d| d.code != Some(DiagnosticCode::OutsideVault))
                .cloned()
                .map(|d| diagnostic(&route.source, &text, analysis.sources(), d)),
        );
        let evaluation = vault.evaluate(&route.source, &analysis)?;
        errors.extend(
            evaluation
                .diagnostics
                .into_iter()
                .map(|d| diagnostic(&route.source, &text, analysis.sources(), d)),
        );
        documents.push(Document::new(
            route,
            evaluation.root,
            text,
            analysis.sources().clone(),
        )?);
    }
    if !errors.is_empty() {
        return Err(Error::Diagnostics(errors));
    }
    let mut assets = Assets::new(output);
    outputs.extend(assets::prepare(
        vault.resources(),
        config,
        &content_assets,
        &mut assets,
    )?);
    let highlight_css = PathBuf::from("_notist/highlight.css");
    assets.generated(
        highlight_css.clone(),
        notist_html::highlight::CSS.as_bytes().to_vec(),
    )?;
    let engine = templates::load(vault.resources(), config)?;
    let data = input.data;
    let by_source: BTreeMap<_, _> = documents
        .iter()
        .map(|d| (d.page.route.source.clone(), d))
        .collect();
    let mut warnings = vec![];
    let mut pages = vec![];
    let mut components = BTreeMap::new();
    for document in &documents {
        let mut resolved = BTreeMap::new();
        for node in document
            .root
            .descendants()
            .filter(|n| matches!(n.ctor, Ctor::Link | Ctor::Embed))
        {
            let Some(Value::Str(target)) = node.fields.get("target") else {
                continue;
            };
            match resolve_link(document, target, node, &content, &by_source, &outputs) {
                Ok(Some(url)) => {
                    resolved.insert(
                        (
                            node.origin.as_ref().map(|o| o.source.clone()),
                            target.clone(),
                        ),
                        url,
                    );
                }
                Ok(None) => {}
                Err(message) => {
                    let d = issue(document, node, message);
                    if node.ctor == Ctor::Embed {
                        errors.push(d);
                    } else {
                        warnings.push(d);
                    }
                }
            }
        }
        let lookup = |node: &Item, url: &str| {
            resolved
                .get(&(
                    node.origin.as_ref().map(|o| o.source.clone()),
                    url.to_owned(),
                ))
                .cloned()
        };
        let registry = vault.html_registry(
            &document.page.route.source,
            &document.source,
            &document.root,
        )?;
        let rendered = notist_html::Renderer::new()
            .with_registry(registry)
            .with_code_renderer(notist_html::highlight::code)
            .with_url_resolver(lookup)
            .with_embed_renderer(|node| {
                let Some(Value::Str(target)) = node.fields.get("target") else {
                    return None;
                };
                image(
                    node,
                    &lookup(node, target).unwrap_or_else(|| target.clone()),
                )
            })
            .render_with_diagnostics(&document.root);
        errors.extend(rendered.diagnostics.into_iter().map(|d| {
            diagnostic(
                &document.page.route.source,
                &document.source,
                &document.sources,
                d,
            )
        }));
        let environment = vault.environment_for(&document.page.route.source, &document.source)?;
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
        let mut page = document.page.clone();
        page.html = rendered.html;
        pages.push(page);
    }
    if !errors.is_empty() {
        return Err(Error::Diagnostics(errors));
    }
    let dirs = directories(&pages, &source_dirs, &content);
    let site = serde_json::json!({ "title": config.title, "base_url": config.base_url, "pages": pages, "directories": dirs, "data": data });
    for page in &pages {
        let template = page
            .attrs
            .get("template")
            .and_then(|v| v.as_str())
            .unwrap_or(&config.template);
        let context = serde_json::json!({ "site": site, "page": page, "directory": dirs[&page.directory], "assets": {
            "components": components.get(&page.route.path),
            "highlight": relative_url(&page.route.output, &highlight_css, false),
        } });
        let html = engine
            .render(template, &context)
            .map_err(|e| Error::Message(format!("{}: {e}", page.route.path.display())))?;
        assets.generated(page.route.output.clone(), html.into_bytes())?;
    }
    Ok(BuildOutput {
        files: assets.finish()?,
        pages,
        warnings,
    })
}
