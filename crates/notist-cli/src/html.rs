//! Single-document HTML export, independent of book structure and themes.
use notist::{
    Environment, Resources,
    publication::{Assets, Error},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub fn assemble(
    root: &notist::Item,
    sources: &notist::Sources,
    registry: notist_html::HtmlRegistry,
    environment: &Environment,
    resources: &impl Resources,
    source: &Path,
    output: &Path,
) -> Result<BTreeMap<PathBuf, Vec<u8>>, Error> {
    let output = notist::resources::normalize(
        &std::env::current_dir()
            .map_err(|error| Error::Message(error.to_string()))?
            .join(output),
    );
    let mut assets = Assets::new(output);
    let mut resource_urls = BTreeMap::new();
    for item in notist::embed::references(root) {
        let Some(notist::Value::Path(path)) = item.field("target") else {
            continue;
        };
        if resource_urls.contains_key(path) {
            continue;
        }
        let source = PathBuf::from(path.logical_path());
        let relative = source
            .strip_prefix(resources.root())
            .map_err(|_| Error::Message("embedded resource is outside the Vault root".into()))?;
        // An embed publishes one file, never an implicit recursive directory.
        if resources.kind(&source)? != Some(notist::resources::ResourceKind::File) {
            return Err(Error::Message(format!(
                "missing embedded resource or not a file: {}",
                source.display()
            )));
        }
        let destination = Path::new("_notist/resources").join(relative);
        assets.copy(resources, &source, &destination)?;
        resource_urls.insert(
            path.clone(),
            notist::publication::url::relative_url(Path::new("index.html"), &destination, false),
        );
    }
    let rendered = notist_html::Renderer::from_registry(registry)
        .with_sources(sources)
        .with_resource_resolver(|_, path| resource_urls.get(path).cloned())
        .render_with_diagnostics(root);
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
    let registrations = assets.components(
        resources,
        environment,
        &rendered.used_components,
        Path::new("components.js"),
    )?;
    let script = if registrations.is_empty() {
        ""
    } else {
        assets.generated("components.js".into(), registrations.into_bytes())?;
        "<script type=\"module\" src=\"./components.js\"></script>"
    };
    let title = notist_html::escape_text(
        source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("Document"),
    );
    assets.generated(
        "_notist/document.css".into(),
        include_bytes!("html.css").to_vec(),
    )?;
    assets.generated(
        "_notist/status.css".into(),
        notist_html::STATUS_CSS.as_bytes().to_vec(),
    )?;
    assets.generated("index.html".into(), format!("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title}</title><link rel=\"stylesheet\" href=\"./_notist/document.css\"><link rel=\"stylesheet\" href=\"./_notist/status.css\"></head><body><main class=\"prose\">{}{script}</main></body></html>", rendered.html).into_bytes())?;
    assets.finish()
}
