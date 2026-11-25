use crate::Error;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Paths are relative to the resource root. Content paths map directly to HTML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SiteConfig {
    pub title: String,
    pub base_url: String,
    pub content: PathBuf,
    pub output: PathBuf,
    /// A presentation package containing templates/, static/ and optional sass/.
    pub theme: Option<PathBuf>,
    /// Site templates override theme templates by relative name.
    pub templates: PathBuf,
    /// Additional assets copied to the output root.
    pub static_dir: PathBuf,
    /// TOML and JSON files exposed by extensionless relative path in site.data.
    pub data: PathBuf,
    /// Top-level .scss/.sass files are compiled; partials remain inputs only.
    pub sass: PathBuf,
    pub template: String,
    /// Experimental site input extensions, in execution order.
    pub extensions: crate::extensions::ExtensionsConfig,
    /// Explicit permutation of configured extension names; defaults to name order.
    pub extension_order: Vec<String>,
    /// Content-relative source filters, applied before site extensions.
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Directory URLs map README and index sources to their parent directory.
    pub directory_urls: bool,
    pub mounts: Vec<Mount>,
    /// Copy every non-document content file. Disable for mixed/private source trees.
    pub copy_content_assets: bool,
    pub styles: Vec<PathBuf>,
    pub scripts: Vec<PathBuf>,
}
impl Default for SiteConfig {
    fn default() -> Self {
        Self {
            title: "Site".into(),
            base_url: "/".into(),
            content: "content".into(),
            output: "public".into(),
            theme: None,
            templates: "templates".into(),
            static_dir: "static".into(),
            data: "data".into(),
            sass: "sass".into(),
            template: "page".into(),
            extensions: Default::default(),
            extension_order: Vec::new(),
            include: vec!["**".into()],
            exclude: Vec::new(),
            directory_urls: false,
            mounts: Vec::new(),
            copy_content_assets: true,
            styles: Vec::new(),
            scripts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    pub source: PathBuf,
    pub path: PathBuf,
}
impl SiteConfig {
    /// Validate typed target options; project manifests are parsed by the host.
    pub fn validate(&self) -> Result<(), Error> {
        let config = self;
        for (name, path) in [
            ("content", &config.content),
            ("output", &config.output),
            ("templates", &config.templates),
            ("static_dir", &config.static_dir),
            ("data", &config.data),
            ("sass", &config.sass),
        ] {
            if path.as_os_str().is_empty() {
                return Err(Error::Message(format!("site {name} must not be empty")));
            }
        }
        if config.template.is_empty() {
            return Err(Error::Message("site template must not be empty".into()));
        }
        for (name, extension) in &config.extensions.commands {
            if name.is_empty() || name.contains('.') || name == "builtins" {
                return Err(Error::Message(format!(
                    "invalid site extension name `{name}`"
                )));
            }
            let command = &extension.command;
            if command.first().is_none_or(|program| program.is_empty()) {
                return Err(Error::Message(
                    "site extension command requires a nonempty program".into(),
                ));
            }
        }
        if !config.extension_order.is_empty() {
            let names = config.extensions.names();
            let order: std::collections::BTreeSet<_> =
                config.extension_order.iter().cloned().collect();
            if names != order || order.len() != config.extension_order.len() {
                return Err(Error::Message(
                    "extension_order must list every configured extension exactly once".into(),
                ));
            }
        }
        if !config.base_url.starts_with('/') || config.base_url.starts_with("//") {
            let url = url::Url::parse(&config.base_url)
                .map_err(|e| Error::Message(format!("invalid site base_url: {e}")))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(Error::Message(
                    "site base_url must be an HTTP(S) URL or a root-relative path".into(),
                ));
            }
        } else if config.base_url.contains(['?', '#', '\\']) {
            return Err(Error::Message("invalid site base_url path".into()));
        }
        Ok(())
    }
    pub(crate) fn url(&self, path: &str) -> String {
        if path.starts_with("//") || path.split('/').next().is_some_and(|s| s.contains(':')) {
            return path.into();
        }
        format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}
