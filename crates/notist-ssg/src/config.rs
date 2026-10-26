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
    pub extensions: Vec<crate::extensions::ExtensionConfig>,
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
            extensions: Vec::new(),
        }
    }
}
impl SiteConfig {
    pub fn parse(source: &str) -> Result<Self, Error> {
        #[derive(Deserialize)]
        struct Config {
            #[serde(default)]
            ssg: SiteConfig,
        }
        let config = toml::from_str::<Config>(source)
            .map_err(|e| Error::Message(e.to_string()))?
            .ssg;
        for (name, path) in [
            ("content", &config.content),
            ("output", &config.output),
            ("templates", &config.templates),
            ("static_dir", &config.static_dir),
            ("data", &config.data),
            ("sass", &config.sass),
        ] {
            if path.as_os_str().is_empty() {
                return Err(Error::Message(format!("ssg {name} must not be empty")));
            }
        }
        if config.template.is_empty() {
            return Err(Error::Message("ssg template must not be empty".into()));
        }
        for extension in &config.extensions {
            let crate::extensions::ExtensionConfig::Command { command, .. } = extension;
            if command.first().is_none_or(|program| program.is_empty()) {
                return Err(Error::Message(
                    "ssg extension command requires a nonempty program".into(),
                ));
            }
        }
        if !config.base_url.starts_with('/') || config.base_url.starts_with("//") {
            let url = url::Url::parse(&config.base_url)
                .map_err(|e| Error::Message(format!("invalid ssg base_url: {e}")))?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(Error::Message(
                    "ssg base_url must be an HTTP(S) URL or a root-relative path".into(),
                ));
            }
        } else if config.base_url.contains(['?', '#', '\\']) {
            return Err(Error::Message("invalid ssg base_url path".into()));
        }
        Ok(config)
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
