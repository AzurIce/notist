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
    rendered: notist_html::RenderResult,
    environment: &Environment,
    resources: &impl Resources,
    source: &Path,
    output: &Path,
) -> Result<BTreeMap<PathBuf, Vec<u8>>, Error> {
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
    let output = notist::resources::normalize(
        &std::env::current_dir()
            .map_err(|error| Error::Message(error.to_string()))?
            .join(output),
    );
    let mut assets = Assets::new(output);
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
