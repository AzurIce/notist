//! Owned host input suitable for transport to a Worker. Asset bytes may be
//! omitted when the host only publishes URLs and performs no resource copying.
use crate::resources::{ResourceError, ResourceKind, ResourceReader, ResourceRevision};
use crate::{InputError, InputRead, MemoryResources, Resources, Vault};
use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

/// Confirmed host preparation failures, without source-specific diagnostics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum PreparedFailure {
    NotFound,
    AccessDenied,
    NotFile,
    TooLarge,
    Io(String),
}
impl From<PreparedFailure> for InputError {
    fn from(error: PreparedFailure) -> Self {
        match error {
            PreparedFailure::NotFound => Self::NotFound,
            PreparedFailure::AccessDenied => Self::AccessDenied,
            PreparedFailure::NotFile => Self::NotFile,
            PreparedFailure::TooLarge => Self::TooLarge,
            PreparedFailure::Io(message) => Self::Io(message),
        }
    }
}

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
    #[serde(default)]
    pub failures: BTreeMap<PathBuf, PreparedFailure>,
    /// Missing allowed data inputs request preparation rather than implying 404.
    #[serde(default)]
    pub incomplete: bool,
    /// None preserves the project's explicit default root grant. Some replaces
    /// it; module imports and URL publication mappings do not add read grants.
    #[serde(default)]
    pub read_roots: Option<Vec<PathBuf>>,
}
impl Default for PreparedInputs {
    fn default() -> Self {
        Self {
            root: "/preview".into(),
            config: None,
            files: BTreeMap::new(),
            module_urls: BTreeMap::new(),
            failures: BTreeMap::new(),
            incomplete: false,
            read_roots: None,
        }
    }
}
impl PreparedInputs {
    pub fn into_vault(self) -> Vault<PreparedResources> {
        let mut resources = MemoryResources::new(self.root);
        for (path, contents) in self.files {
            resources.insert(path, contents);
        }
        let failures = self
            .failures
            .into_iter()
            .map(|(path, error)| (resources.resolve(&path), error))
            .collect();
        let urls = self
            .module_urls
            .into_iter()
            .map(|(path, url)| (resources.resolve(&path), url))
            .collect::<BTreeMap<_, _>>();
        let mut vault = Vault::new(PreparedResources {
            files: resources,
            failures: Arc::new(failures),
            urls: Arc::new(urls.clone()),
            incomplete: self.incomplete,
        })
        .with_module_urls(urls);
        if let Some(roots) = self.read_roots {
            vault = vault.with_read_roots(roots);
        }
        if let Some(config) = self.config {
            vault = vault.with_config(config);
        }
        vault
    }
}

/// Immutable browser input snapshot. Publication availability is distinct from
/// having bytes, and confirmed failures are distinct from unprepared resources.
#[derive(Clone)]
pub struct PreparedResources {
    files: MemoryResources,
    failures: Arc<BTreeMap<PathBuf, PreparedFailure>>,
    urls: Arc<BTreeMap<PathBuf, String>>,
    incomplete: bool,
}
impl Resources for PreparedResources {
    fn root(&self) -> &Path {
        self.files.root()
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        self.files.read(path)
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        let path = self.resolve(path);
        if self.urls.contains_key(&path) {
            return Ok(Some(ResourceKind::File));
        }
        if self.urls.keys().any(|p| p.starts_with(&path)) {
            return Ok(Some(ResourceKind::Directory));
        }
        self.files.kind(&path)
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        let path = self.resolve(path);
        let mut entries = self.files.entries(&path).or_else(|error| match error {
            ResourceError::NotFound(_) if self.kind(&path)? == Some(ResourceKind::Directory) => {
                Ok(vec![])
            }
            error => Err(error),
        })?;
        for file in self.urls.keys() {
            if let Ok(relative) = file.strip_prefix(&path)
                && let Some(first) = relative.components().next()
            {
                entries.push(path.join(first));
            }
        }
        entries.sort();
        entries.dedup();
        Ok(entries)
    }
    fn read_root(&self, root: &Path) -> Result<Arc<dyn ResourceReader>, InputError> {
        Ok(Arc::new(PreparedReader {
            root: self.resolve(root),
            resources: self.clone(),
            base: self.files.read_root(root)?,
        }))
    }
}
struct PreparedReader {
    root: PathBuf,
    resources: PreparedResources,
    base: Arc<dyn ResourceReader>,
}
impl PreparedReader {
    fn state(&self, path: &Path) -> Result<Option<InputRead>, InputError> {
        if path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(InputError::AccessDenied);
        }
        let path = self.root.join(path);
        // Explicit bytes win over a stale preparation error in transported data.
        if self.resources.files.files().contains_key(&path) {
            return Ok(None);
        }
        if let Some(error) = self.resources.failures.get(&path) {
            return Ok(Some(InputRead::Failed(error.clone().into())));
        }
        if self
            .resources
            .files
            .kind(&path)
            .map_err(|e| InputError::Io(e.to_string()))?
            == Some(ResourceKind::Directory)
        {
            return Ok(Some(InputRead::Failed(InputError::NotFile)));
        }
        Ok(
            (self.resources.incomplete || self.resources.urls.contains_key(&path))
                .then_some(InputRead::Pending),
        )
    }
}
impl ResourceReader for PreparedReader {
    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead {
        match self.state(path) {
            Ok(Some(read)) => read,
            Ok(None) => self.base.read_bounded(path, limit),
            Err(error) => InputRead::Failed(error),
        }
    }
    fn revision(&self, path: &Path) -> Result<Option<ResourceRevision>, InputError> {
        match self.state(path)? {
            Some(InputRead::Failed(error)) => Err(error),
            Some(_) => Ok(None),
            None => self.base.revision(path),
        }
    }
}
