use serde::{Deserialize, de::Error as _};
use std::path::{Path, PathBuf};

/// Book presentation and chapter discovery, independent of document scopes.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BookConfig {
    #[serde(default = "default_title")]
    pub title: String,
    #[serde(default = "default_include")]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    /// Explicit output directory; relative paths are resolved against the Vault.
    pub output: PathBuf,
    /// Vault-relative directory containing optional index.hbs and assets/.
    pub theme: Option<PathBuf>,
    /// Vault-relative stylesheets, loaded last in the declared order.
    #[serde(default, rename = "additional-css")]
    pub additional_css: Vec<PathBuf>,
    /// Vault-relative classic scripts, deferred after the theme script.
    #[serde(default, rename = "additional-js")]
    pub additional_js: Vec<PathBuf>,
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
            output: output.into(),
            theme: None,
            additional_css: vec![],
            additional_js: vec![],
        }
    }
    /// Read only [book]; module configuration remains owned by notist.
    /// An explicit host output overrides [book].output, including when absent.
    pub fn parse(source: &str, output: Option<&Path>) -> Result<Self, toml::de::Error> {
        #[derive(Deserialize)]
        struct Config {
            #[serde(default)]
            book: toml::Table,
        }
        let mut book = toml::from_str::<Config>(source)?.book;
        if let Some(output) = output {
            book.insert("output".into(), output.to_string_lossy().as_ref().into());
        }
        if !book.contains_key("output") {
            return Err(toml::de::Error::custom(
                "missing book output: set [book].output or pass --out-dir",
            ));
        }
        let mut config: Self = toml::Value::Table(book).try_into()?;
        if let Some(output) = output {
            // Preserve OS paths that cannot round-trip through a TOML string.
            config.output = output.into();
        }
        if config.output.as_os_str().is_empty() {
            return Err(toml::de::Error::custom("book output must not be empty"));
        }
        Ok(config)
    }
}
