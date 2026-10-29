use serde::Deserialize;
use std::path::PathBuf;

/// Book presentation and eligible chapter sources, independent of document scopes.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookConfig {
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default = "default_include")]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// External source directories mounted into the book's path namespace.
    #[serde(default)]
    pub mounts: Vec<Mount>,
    /// Explicit output directory; relative paths are resolved against the Vault.
    pub output: PathBuf,
    /// Book source directory containing optional index.hbs and assets/.
    pub theme: Option<PathBuf>,
    /// Book source stylesheets, loaded last in the declared order.
    #[serde(default, rename = "additional-css")]
    pub additional_css: Vec<PathBuf>,
    /// Book source classic scripts, deferred after the theme script.
    #[serde(default, rename = "additional-js")]
    pub additional_js: Vec<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    /// Directory relative to the Vault root (or an absolute source path).
    pub source: PathBuf,
    /// Nonempty book-relative directory used for filtering and publication.
    pub path: PathBuf,
}
fn default_title() -> String {
    "Book".into()
}
fn default_include() -> Vec<String> {
    vec!["**".into()]
}
impl BookConfig {
    pub fn new(output: impl Into<PathBuf>) -> Self {
        Self {
            title: default_title(),
            include: default_include(),
            exclude: vec![],
            mounts: vec![],
            output: output.into(),
            theme: None,
            additional_css: vec![],
            additional_js: vec![],
        }
    }
}
