//! Owned host input suitable for transport to a Worker. Asset bytes may be
//! omitted when the host only publishes URLs and performs no resource copying.
use crate::{MemoryResources, Vault};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedInputs {
    /// Absolute logical root, independent of the host's physical filesystem.
    pub root: PathBuf,
    #[serde(default)]
    pub config: Option<PathBuf>,
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    #[serde(default)]
    pub module_urls: BTreeMap<PathBuf, String>,
}
impl PreparedInputs {
    pub fn into_vault(self) -> Vault<MemoryResources> {
        let mut resources = MemoryResources::new(self.root);
        for (path, contents) in self.files {
            resources.insert(path, contents);
        }
        let mut vault = Vault::new(resources).with_module_urls(self.module_urls);
        if let Some(config) = self.config {
            vault = vault.with_config(config);
        }
        vault
    }
}
