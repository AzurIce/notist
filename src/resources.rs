//! Read-only logical resources. Async hosts can prepare a MemoryResources snapshot.
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceError {
    NotFound(PathBuf),
    InvalidUtf8(PathBuf),
    Access { path: PathBuf, message: String },
}
impl std::fmt::Display for ResourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(path) => write!(f, "resource not found: {}", path.display()),
            Self::InvalidUtf8(path) => write!(f, "resource is not UTF-8: {}", path.display()),
            Self::Access { path, message } => write!(f, "{}: {message}", path.display()),
        }
    }
}
impl std::error::Error for ResourceError {}

/// Paths are logical resource identities, never browser URLs. Calls within a
/// task must observe a consistent input snapshot. Missing entries are distinct
/// from access failures. There is no write or browser execution capability.
pub trait Resources {
    /// An absolute logical root; resolved identities must be stable on re-resolution.
    fn root(&self) -> &Path;
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError>;
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError>;
    fn entries(&self, directory: &Path) -> Result<Vec<PathBuf>, ResourceError>;
    fn resolve(&self, path: &Path) -> PathBuf {
        normalize(&self.root().join(path))
    }
    fn source(&self, path: &Path) -> Result<String, ResourceError> {
        String::from_utf8(self.read(path)?)
            .map_err(|_| ResourceError::InvalidUtf8(self.resolve(path)))
    }
}

impl<R: Resources + ?Sized> Resources for &R {
    fn root(&self) -> &Path {
        (**self).root()
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        (**self).read(path)
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        (**self).kind(path)
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        (**self).entries(path)
    }
    fn resolve(&self, path: &Path) -> PathBuf {
        (**self).resolve(path)
    }
}

/// A prepared, owned resource snapshot suitable for an analysis Worker.
#[derive(Debug, Clone)]
pub struct MemoryResources {
    root: PathBuf,
    files: BTreeMap<PathBuf, Vec<u8>>,
}
impl MemoryResources {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: normalize(&Path::new("/").join(root)),
            files: BTreeMap::new(),
        }
    }
    pub fn insert(&mut self, path: impl AsRef<Path>, contents: impl Into<Vec<u8>>) {
        self.files
            .insert(self.resolve(path.as_ref()), contents.into());
    }
    pub fn files(&self) -> &BTreeMap<PathBuf, Vec<u8>> {
        &self.files
    }
}
impl Resources for MemoryResources {
    fn root(&self) -> &Path {
        &self.root
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        let path = self.resolve(path);
        match self.files.get(&path) {
            Some(contents) => Ok(contents.clone()),
            None if self.kind(&path)? == Some(ResourceKind::Directory) => {
                Err(ResourceError::Access {
                    path,
                    message: "cannot read a directory".into(),
                })
            }
            None => Err(ResourceError::NotFound(path)),
        }
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        let path = self.resolve(path);
        Ok(if self.files.contains_key(&path) {
            Some(ResourceKind::File)
        } else if path == self.root || self.files.keys().any(|file| file.starts_with(&path)) {
            Some(ResourceKind::Directory)
        } else {
            None
        })
    }
    fn entries(&self, directory: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        let directory = self.resolve(directory);
        match self.kind(&directory)? {
            None => return Err(ResourceError::NotFound(directory)),
            Some(ResourceKind::File) => {
                return Err(ResourceError::Access {
                    path: directory,
                    message: "not a directory".into(),
                });
            }
            Some(ResourceKind::Directory) => {}
        }
        let mut entries = std::collections::BTreeSet::new();
        for path in self.files.keys() {
            if let Ok(relative) = path.strip_prefix(&directory) {
                if let Some(first) = relative.components().next() {
                    entries.insert(directory.join(first));
                }
            }
        }
        Ok(entries.into_iter().collect())
    }
}

/// Filesystem adapter. Its root determines relative-path resolution, not a
/// sandbox; explicit dependency paths may refer to sibling directories.
#[derive(Debug, Clone)]
pub struct FsResources {
    root: PathBuf,
}
impl FsResources {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = std::env::current_dir().unwrap_or_default().join(root);
        Self {
            root: normalize(&root),
        }
    }
}
impl Default for FsResources {
    fn default() -> Self {
        Self::new(".")
    }
}
fn io_error(path: PathBuf, error: std::io::Error) -> ResourceError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ResourceError::NotFound(path)
    } else {
        ResourceError::Access {
            path,
            message: error.to_string(),
        }
    }
}
impl Resources for FsResources {
    fn root(&self) -> &Path {
        &self.root
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        let path = self.resolve(path);
        std::fs::read(&path).map_err(|e| io_error(path, e))
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        let path = self.resolve(path);
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() => Ok(Some(ResourceKind::File)),
            Ok(meta) if meta.is_dir() => Ok(Some(ResourceKind::Directory)),
            Ok(_) => Err(ResourceError::Access {
                path,
                message: "not a regular file or directory".into(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error(path, e)),
        }
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        let path = self.resolve(path);
        let mut entries = std::fs::read_dir(&path)
            .map_err(|e| io_error(path.clone(), e))?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|e| io_error(path.clone(), e))
            })
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort();
        Ok(entries)
    }
}

/// Unsaved text takes precedence over the underlying resources, including a
/// configuration or declaration that does not yet exist on disk.
pub struct OverlayResources<'a, R: ?Sized> {
    base: &'a R,
    sources: BTreeMap<PathBuf, String>,
}
impl<'a, R: Resources + ?Sized> OverlayResources<'a, R> {
    pub fn new(base: &'a R, sources: &BTreeMap<PathBuf, String>) -> Self {
        Self {
            base,
            sources: sources
                .iter()
                .map(|(path, source)| (base.resolve(path), source.clone()))
                .collect(),
        }
    }
}
impl<R: Resources + ?Sized> Resources for OverlayResources<'_, R> {
    fn root(&self) -> &Path {
        self.base.root()
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        let path = self.resolve(path);
        match self.sources.get(&path) {
            Some(source) => Ok(source.as_bytes().to_vec()),
            None if self.kind(&path)? == Some(ResourceKind::Directory) => {
                Err(ResourceError::Access {
                    path,
                    message: "cannot read a directory".into(),
                })
            }
            None => self.base.read(&path),
        }
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        let path = self.resolve(path);
        if self.sources.contains_key(&path) {
            Ok(Some(ResourceKind::File))
        } else if self.sources.keys().any(|source| source.starts_with(&path)) {
            Ok(Some(ResourceKind::Directory))
        } else {
            self.base.kind(&path)
        }
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        let path = self.resolve(path);
        let mut entries = match self.base.kind(&path)? {
            Some(ResourceKind::Directory) => self.base.entries(&path)?,
            _ => Vec::new(),
        };
        match self.kind(&path)? {
            None => return Err(ResourceError::NotFound(path)),
            Some(ResourceKind::File) => {
                return Err(ResourceError::Access {
                    path,
                    message: "not a directory".into(),
                });
            }
            Some(ResourceKind::Directory) => {}
        }
        for source in self.sources.keys() {
            if let Ok(relative) = source.strip_prefix(&path) {
                if let Some(first) = relative.components().next() {
                    entries.push(path.join(first));
                }
            }
        }
        entries.sort();
        entries.dedup();
        Ok(entries)
    }
}

/// Normalize logical paths without reading the filesystem.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                if out.file_name().is_some_and(|name| name != "..") {
                    out.pop();
                } else if !out.has_root() {
                    out.push("..");
                }
            }
            Component::CurDir => {}
            component => out.push(component.as_os_str()),
        }
    }
    out
}
