//! Read-only logical resources. Async hosts can prepare a MemoryResources snapshot.
use crate::{InputError, InputRead};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// A host-created, rooted reader. Paths are relative to this capability's root.
/// Implementations must enforce that boundary and bound allocation before
/// reading/copying. ResourceSession fixes the result of the first read.
pub trait ResourceReader: Send + Sync {
    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead;
    /// Verify actual bytes through this same rooted capability. Implementations
    /// may stream without retaining a second full file. Failure is never proof
    /// that an output is unchanged; the default preserves bounded-reader hosts.
    fn matches_bytes(
        &self,
        path: &Path,
        expected: &[u8],
        buffer: &mut [u8],
    ) -> Result<bool, InputError> {
        if buffer.is_empty() {
            return Err(InputError::Io("empty comparison buffer".into()));
        }
        match self.read_bounded(path, expected.len()) {
            InputRead::Ready(actual) => Ok(actual.as_ref() == expected),
            InputRead::Failed(error) => Err(error),
            InputRead::Pending => Err(InputError::Unavailable),
        }
    }
}

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

/// Paths are logical resource identities, never browser URLs. ResourceSession
/// caches reads within one batch. Missing entries are distinct
/// from access failures. There is no write or browser execution capability.
pub trait Resources {
    /// An absolute logical root; resolved identities must be stable on re-resolution.
    fn root(&self) -> &Path;
    /// Lexical Path root for a loaded source; this conveys no read permission.
    fn source_root(&self, path: &Path) -> PathBuf {
        if path.starts_with(self.root()) {
            self.root().into()
        } else {
            path.parent().unwrap_or(self.root()).into()
        }
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError>;
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError>;
    fn entries(&self, directory: &Path) -> Result<Vec<PathBuf>, ResourceError>;
    /// Explicit bounded data capability. Legacy backends remain source-only;
    /// there is intentionally no default that calls the unbounded `read`.
    fn read_root(&self, _root: &Path) -> Result<Arc<dyn ResourceReader>, InputError> {
        Err(InputError::Unavailable)
    }
    fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            normalize(path)
        } else {
            normalize(&self.root().join(path))
        }
    }
    fn source(&self, path: &Path) -> Result<String, ResourceError> {
        String::from_utf8(self.read(path)?)
            .map_err(|_| ResourceError::InvalidUtf8(self.resolve(path)))
    }
    /// Shared UTF-8 source text. Hosts may retain this within their input batch.
    fn source_shared(&self, path: &Path) -> Result<Arc<str>, ResourceError> {
        self.source(path).map(Arc::from)
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
    fn read_root(&self, root: &Path) -> Result<Arc<dyn ResourceReader>, InputError> {
        (**self).read_root(root)
    }
    fn source_root(&self, path: &Path) -> PathBuf {
        (**self).source_root(path)
    }
    fn source_shared(&self, path: &Path) -> Result<Arc<str>, ResourceError> {
        (**self).source_shared(path)
    }
}

/// A prepared, owned resource snapshot suitable for an analysis Worker.
#[derive(Debug, Clone)]
pub struct MemoryResources {
    root: PathBuf,
    files: Arc<BTreeMap<PathBuf, Vec<u8>>>,
}
impl MemoryResources {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: normalize(&Path::new("/").join(root)),
            files: Arc::default(),
        }
    }
    pub fn insert(&mut self, path: impl AsRef<Path>, contents: impl Into<Vec<u8>>) {
        let path = self.resolve(path.as_ref());
        Arc::make_mut(&mut self.files).insert(path, contents.into());
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
            if let Ok(relative) = path.strip_prefix(&directory)
                && let Some(first) = relative.components().next()
            {
                entries.insert(directory.join(first));
            }
        }
        Ok(entries.into_iter().collect())
    }
    fn read_root(&self, root: &Path) -> Result<Arc<dyn ResourceReader>, InputError> {
        Ok(Arc::new(MemoryReader {
            root: self.resolve(root),
            files: self.files.clone(),
        }))
    }
}

struct MemoryReader {
    root: PathBuf,
    files: Arc<BTreeMap<PathBuf, Vec<u8>>>,
}
impl MemoryReader {
    fn bytes(&self, path: &Path) -> Result<&[u8], InputError> {
        let path = within(&self.root, path)?;
        self.files.get(&path).map(Vec::as_slice).ok_or_else(|| {
            if path == self.root || self.files.keys().any(|p| p.starts_with(&path)) {
                InputError::NotFile
            } else {
                InputError::NotFound
            }
        })
    }
}
impl ResourceReader for MemoryReader {
    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead {
        match self.bytes(path) {
            Ok(bytes) if bytes.len() <= limit => InputRead::Ready(Arc::from(bytes)),
            Ok(_) => InputRead::Failed(InputError::TooLarge),
            Err(error) => InputRead::Failed(error),
        }
    }
}

fn within(root: &Path, relative: &Path) -> Result<PathBuf, InputError> {
    validate_relative(relative)?;
    Ok(root.join(relative))
}

fn validate_relative(relative: &Path) -> Result<(), InputError> {
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(InputError::AccessDenied);
    }
    Ok(())
}

/// Filesystem adapter. Its root determines relative-path resolution, not a
/// sandbox; explicit dependency paths may refer to sibling directories.
#[derive(Debug, Clone)]
pub struct FsResources {
    root: PathBuf,
}
/// An already resolved lexical filesystem identity. This carries no grant,
/// kind, metadata, or byte snapshot and never bypasses a reader capability.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResolvedPath(PathBuf);
impl ResolvedPath {
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}
impl FsResources {
    pub fn new(root: impl AsRef<Path>) -> Self {
        let root = std::env::current_dir().unwrap_or_default().join(root);
        Self {
            root: normalize(&root),
        }
    }
    pub fn resolve_path(&self, path: &Path) -> ResolvedPath {
        ResolvedPath(self.resolve(path))
    }
    pub fn read_resolved(&self, path: &ResolvedPath) -> Result<Vec<u8>, ResourceError> {
        std::fs::read(path.as_path()).map_err(|e| io_error(path.0.clone(), e))
    }
    pub fn kind_resolved(
        &self,
        path: &ResolvedPath,
    ) -> Result<Option<ResourceKind>, ResourceError> {
        match std::fs::metadata(path.as_path()) {
            Ok(meta) if meta.is_file() => Ok(Some(ResourceKind::File)),
            Ok(meta) if meta.is_dir() => Ok(Some(ResourceKind::Directory)),
            Ok(_) => Err(ResourceError::Access {
                path: path.0.clone(),
                message: "not a regular file or directory".into(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error(path.0.clone(), e)),
        }
    }
    pub fn entries_resolved(&self, path: &ResolvedPath) -> Result<Vec<PathBuf>, ResourceError> {
        let mut entries = std::fs::read_dir(path.as_path())
            .map_err(|e| io_error(path.0.clone(), e))?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|e| io_error(path.0.clone(), e))
            })
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort();
        Ok(entries)
    }
    pub fn read_root_resolved(
        &self,
        root: &ResolvedPath,
    ) -> Result<Arc<dyn ResourceReader>, InputError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let directory =
                cap_std::fs::Dir::open_ambient_dir(root.as_path(), cap_std::ambient_authority())
                    .map_err(input_io)?;
            Ok(Arc::new(FsReader(directory)))
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = root;
            Err(InputError::Unavailable)
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
        self.read_resolved(&self.resolve_path(path))
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        self.kind_resolved(&self.resolve_path(path))
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        self.entries_resolved(&self.resolve_path(path))
    }
    fn read_root(&self, root: &Path) -> Result<Arc<dyn ResourceReader>, InputError> {
        self.read_root_resolved(&self.resolve_path(root))
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn input_io(error: std::io::Error) -> InputError {
    match error.kind() {
        std::io::ErrorKind::NotFound => InputError::NotFound,
        std::io::ErrorKind::PermissionDenied => InputError::AccessDenied,
        std::io::ErrorKind::IsADirectory | std::io::ErrorKind::NotADirectory => InputError::NotFile,
        _ => InputError::Io(error.to_string()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct FsReader(cap_std::fs::Dir);
#[cfg(not(target_arch = "wasm32"))]
impl FsReader {
    fn checked_file(
        &self,
        path: &Path,
        limit: usize,
    ) -> Result<(cap_std::fs::File, cap_std::fs::Metadata), InputError> {
        validate_relative(path)?;
        if path.as_os_str().is_empty() {
            return Err(InputError::NotFile);
        }
        let mut options = cap_std::fs::OpenOptions::new();
        options.read(true);
        // Opening a FIFO must not block before the regular-file check.
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY);
        }
        let file = self.0.open_with(path, &options).map_err(input_io)?;
        let metadata = file.metadata().map_err(input_io)?;
        if !metadata.is_file() {
            return Err(InputError::NotFile);
        }
        if metadata.len() > limit as u64 {
            return Err(InputError::TooLarge);
        }
        Ok((file, metadata))
    }
}
#[cfg(not(target_arch = "wasm32"))]
impl ResourceReader for FsReader {
    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead {
        use std::io::Read;
        let read = (|| {
            let (file, metadata) = self.checked_file(path, limit)?;
            let mut bytes = Vec::new();
            (&file)
                .take((limit as u64).saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(input_io)?;
            if bytes.len() > limit {
                return Err(InputError::TooLarge);
            }
            if !same_metadata(&metadata, &file.metadata().map_err(input_io)?) {
                return Err(InputError::Io("file changed while being read".into()));
            }
            Ok(Arc::from(bytes))
        })();
        match read {
            Ok(bytes) => InputRead::Ready(bytes),
            Err(error) => InputRead::Failed(error),
        }
    }
    fn matches_bytes(
        &self,
        path: &Path,
        expected: &[u8],
        buffer: &mut [u8],
    ) -> Result<bool, InputError> {
        use std::io::Read;
        if buffer.is_empty() {
            return Err(InputError::Io("empty comparison buffer".into()));
        }
        let (mut file, metadata) = self.checked_file(path, expected.len())?;
        if metadata.len() != expected.len() as u64 {
            return Ok(false);
        }
        for chunk in expected.chunks(buffer.len()) {
            file.read_exact(&mut buffer[..chunk.len()])
                .map_err(input_io)?;
            if &buffer[..chunk.len()] != chunk {
                return Ok(false);
            }
        }
        if !same_metadata(&metadata, &file.metadata().map_err(input_io)?) {
            return Err(InputError::Io("file changed while being read".into()));
        }
        Ok(true)
    }
}
#[cfg(not(target_arch = "wasm32"))]
fn same_metadata(a: &cap_std::fs::Metadata, b: &cap_std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        (
            a.len(),
            a.mtime(),
            a.mtime_nsec(),
            a.ctime(),
            a.ctime_nsec(),
        ) == (
            b.len(),
            b.mtime(),
            b.mtime_nsec(),
            b.ctime(),
            b.ctime_nsec(),
        )
    }
    #[cfg(not(unix))]
    {
        a.len() == b.len() && a.modified().ok() == b.modified().ok()
    }
}

/// Unsaved text takes precedence over the underlying resources, including a
/// configuration or declaration that does not yet exist on disk.
pub struct OverlayResources<'a, R: ?Sized> {
    base: &'a R,
    sources: Arc<BTreeMap<PathBuf, String>>,
}
impl<'a, R: Resources + ?Sized> OverlayResources<'a, R> {
    pub fn new(base: &'a R, sources: &BTreeMap<PathBuf, String>) -> Self {
        Self {
            base,
            sources: sources
                .iter()
                .map(|(path, source)| (base.resolve(path), source.clone()))
                .collect::<BTreeMap<_, _>>()
                .into(),
        }
    }
}
impl<R: Resources + ?Sized> Resources for OverlayResources<'_, R> {
    fn root(&self) -> &Path {
        self.base.root()
    }
    fn source_root(&self, path: &Path) -> PathBuf {
        self.base.source_root(path)
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
            if let Ok(relative) = source.strip_prefix(&path)
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
        let root = self.resolve(root);
        Ok(Arc::new(OverlayReader {
            base: self.base.read_root(&root),
            sources: self.sources.clone(),
            root,
        }))
    }
}

struct OverlayReader {
    root: PathBuf,
    sources: Arc<BTreeMap<PathBuf, String>>,
    base: Result<Arc<dyn ResourceReader>, InputError>,
}
impl ResourceReader for OverlayReader {
    fn read_bounded(&self, path: &Path, limit: usize) -> InputRead {
        let absolute = match within(&self.root, path) {
            Ok(path) => path,
            Err(error) => return InputRead::Failed(error),
        };
        match self.sources.get(&absolute) {
            Some(source) if source.len() <= limit => InputRead::Ready(Arc::from(source.as_bytes())),
            Some(_) => InputRead::Failed(InputError::TooLarge),
            None => match &self.base {
                Ok(base) => base.read_bounded(path, limit),
                Err(error) => InputRead::Failed(error.clone()),
            },
        }
    }
}

/// Normalize logical paths without reading the filesystem.
pub fn normalize(path: &Path) -> PathBuf {
    // Normalization removes components; reserve for the input instead of
    // repeatedly growing a buffer while rebuilding the same logical path.
    let mut out = PathBuf::with_capacity(path.as_os_str().len());
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

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn normalization_preserves_lexical_components() {
        for (input, expected) in [
            ("", ""),
            (".", ""),
            ("a//./b/../c/", "a/c"),
            ("../../a/../b", "../../b"),
            ("a/../../b", "../b"),
            ("/../../a/../b", "/b"),
            ("/a/./b/../../", "/"),
        ] {
            assert_eq!(normalize(Path::new(input)), Path::new(expected), "{input}");
        }
    }

    #[test]
    fn resolve_retains_absolute_and_relative_path_semantics() {
        let resources = MemoryResources::new("/project");
        for (input, expected) in [
            ("nested/../file", "/project/file"),
            ("../other/file", "/other/file"),
            ("/other/./file", "/other/file"),
            ("", "/project"),
        ] {
            assert_eq!(resources.resolve(Path::new(input)), Path::new(expected));
        }
    }

    #[cfg(unix)]
    #[test]
    fn normalization_preserves_non_utf8_components() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let input = Path::new(OsStr::from_bytes(b"/a/../\xff/./b"));
        let expected = Path::new(OsStr::from_bytes(b"/\xff/b"));
        assert_eq!(normalize(input), expected);
    }
}
