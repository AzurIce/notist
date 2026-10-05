use serde::Deserialize;
use std::path::PathBuf;

/// Site presentation and discovery, independent of package/environment scopes.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SiteConfig {
    pub title: String,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub output: PathBuf,
    /// Vault-relative directory containing optional index.hbs and assets/.
    pub theme: Option<PathBuf>,
}
impl Default for SiteConfig {
    fn default() -> Self {
        Self {
            title: "Notist".into(),
            include: vec!["**".into()],
            exclude: vec![],
            output: "target/site".into(),
            theme: None,
        }
    }
}
impl SiteConfig {
    /// Read only [site]; package configuration remains owned by notist.
    pub fn parse(source: &str) -> Result<Self, toml::de::Error> {
        #[derive(Deserialize)]
        struct Config {
            #[serde(default)]
            site: SiteConfig,
        }
        toml::from_str::<Config>(source).map(|config| config.site)
    }
}
