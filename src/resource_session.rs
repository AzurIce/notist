//! One lazy input batch: rooted reads, fixed byte/error caches and data grants.
use crate::resources::{ResourceError, ResourceKind, ResourceReader};
use crate::{EvaluationInputs, InputError, InputRead, ResourcePath, Resources};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, Copy)]
pub struct InputLimits {
    pub file_bytes: usize,
    pub cached_bytes: usize,
    pub requests: usize,
}
impl Default for InputLimits {
    fn default() -> Self {
        Self {
            file_bytes: 16 * 1024 * 1024,
            cached_bytes: 128 * 1024 * 1024,
            requests: 16_384,
        }
    }
}

struct ReadRoot {
    path: PathBuf,
    reader: Result<Arc<dyn ResourceReader>, InputError>,
    inputs: RefCell<BTreeMap<PathBuf, InputRead>>,
}
struct CachedInput {
    read: InputRead,
    // Source-only reads do not authorize data access. Reuse bytes only when
    // they were loaded through this exact root capability.
    reader: Option<Arc<dyn ResourceReader>>,
}
#[derive(Default)]
struct State {
    files: BTreeMap<PathBuf, CachedInput>,
    kinds: BTreeMap<PathBuf, Result<Option<ResourceKind>, ResourceError>>,
    sources: BTreeMap<PathBuf, Arc<str>>,
    texts: BTreeMap<PathBuf, Result<Arc<str>, ResourceError>>,
    dependencies: BTreeSet<ResourcePath>,
    bytes: usize,
    requests: usize,
}

/// A lazily loaded input batch. Reads retain their first bytes or error until
/// the host creates a new session; the batch is not an atomic directory snapshot.
/// Data access starts denied and requires explicit roots, even on cache hits.
pub struct ResourceSession<R> {
    resources: R,
    roots: Vec<ReadRoot>,
    limits: InputLimits,
    state: RefCell<State>,
    data_cache: crate::DataCache,
}
impl<R: Resources> ResourceSession<R> {
    pub fn new(resources: R) -> Self {
        Self {
            resources,
            roots: Vec::new(),
            limits: InputLimits::default(),
            state: RefCell::default(),
            data_cache: Default::default(),
        }
    }
    pub fn resources(&self) -> &R {
        &self.resources
    }
    pub fn with_limits(mut self, limits: InputLimits) -> Self {
        self.limits = limits;
        self
    }
    pub fn with_read_root(mut self, root: impl AsRef<Path>) -> Self {
        let path = self.resources.resolve(root.as_ref());
        if !self.roots.iter().any(|r| r.path == path) {
            let reader = self.resources.read_root(&path);
            self.roots.push(ReadRoot {
                path,
                reader,
                inputs: RefCell::default(),
            });
            self.roots
                .sort_by_key(|r| std::cmp::Reverse(r.path.components().count()));
        }
        self
    }
    /// Replace the grant policy before compiling any documents. Byte caches do
    /// not carry authorization: even cached source bytes still require a grant.
    pub fn with_read_roots(mut self, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        self.roots.clear();
        for root in roots {
            self = self.with_read_root(root);
        }
        self
    }
    pub fn dependencies(&self) -> BTreeSet<ResourcePath> {
        self.state.borrow().dependencies.clone()
    }
    pub fn cached_bytes(&self) -> usize {
        self.state.borrow().bytes
    }
    pub fn with_data_cache_limits(mut self, bytes: usize, entries: usize) -> Self {
        self.data_cache = crate::DataCache::new(bytes, entries);
        self
    }
    pub fn data_cache_stats(&self) -> crate::DataCacheStats {
        self.data_cache.stats()
    }
    pub(crate) fn register_source(&self, path: &Path, source: Arc<str>) {
        self.state
            .borrow_mut()
            .sources
            .entry(path.into())
            .or_insert(source);
    }

    fn request(&self) -> Result<(), InputError> {
        let mut state = self.state.borrow_mut();
        state.requests = state.requests.saturating_add(1);
        if state.requests > self.limits.requests {
            Err(InputError::RequestLimit)
        } else {
            Ok(())
        }
    }
    fn grant<'a>(&self, path: &'a Path) -> Result<(&ReadRoot, &'a Path), InputError> {
        let root = self
            .roots
            .iter()
            .find(|root| path.starts_with(&root.path))
            .ok_or(InputError::AccessDenied)?;
        Ok((root, path.strip_prefix(&root.path).unwrap()))
    }
    fn cache(&self, mut read: InputRead) -> InputRead {
        let mut state = self.state.borrow_mut();
        if let InputRead::Ready(bytes) = &read {
            if bytes.len() > self.limits.file_bytes {
                read = InputRead::Failed(InputError::TooLarge);
            } else if state.bytes.saturating_add(bytes.len()) > self.limits.cached_bytes {
                read = InputRead::Failed(InputError::CacheLimit);
            } else {
                state.bytes += bytes.len();
            }
        }
        read
    }
    fn store(
        &self,
        path: &Path,
        read: InputRead,
        reader: Option<Arc<dyn ResourceReader>>,
    ) -> InputRead {
        let read = self.cache(read);
        self.state
            .borrow_mut()
            .files
            .entry(path.into())
            .or_insert_with(|| CachedInput {
                read: read.clone(),
                reader,
            });
        read
    }
    fn load(&self, path: &Path, reader: &Arc<dyn ResourceReader>, relative: &Path) -> InputRead {
        if let Some(cached) = self.state.borrow().files.get(path)
            && cached
                .reader
                .as_ref()
                .is_some_and(|loaded| Arc::ptr_eq(loaded, reader))
        {
            return cached.read.clone();
        }
        let remaining = self
            .limits
            .cached_bytes
            .saturating_sub(self.state.borrow().bytes);
        let limit = self.limits.file_bytes.min(remaining);
        let mut read = reader.read_bounded(relative, limit);
        if matches!(read, InputRead::Failed(InputError::TooLarge)) && limit < self.limits.file_bytes
        {
            read = InputRead::Failed(InputError::CacheLimit);
        }
        self.store(path, read, Some(reader.clone()))
    }
    fn data(&self, path: &ResourcePath) -> InputRead {
        if let Err(error) = self.request() {
            return InputRead::Failed(error);
        }
        self.state.borrow_mut().dependencies.insert(path.clone());
        let absolute = PathBuf::from(path.logical_path());
        let (root, relative) = match self.grant(&absolute) {
            Ok(grant) => grant,
            Err(error) => return InputRead::Failed(error),
        };
        if let Some(read) = root.inputs.borrow().get(&absolute) {
            return read.clone();
        }
        let read = self.read_data(&absolute, root, relative);
        root.inputs.borrow_mut().insert(absolute, read.clone());
        read
    }
    fn read_data(&self, absolute: &Path, root: &ReadRoot, relative: &Path) -> InputRead {
        let source = self.state.borrow().sources.get(absolute).cloned();
        // Explicit document inputs override disk without requiring a real file.
        // Their bytes are allocated and charged only if actually read as data.
        if let Some(source) = source {
            let state = self.state.borrow();
            if let Some(CachedInput {
                read: InputRead::Ready(bytes),
                ..
            }) = state.files.get(absolute)
                && bytes.as_ref() == source.as_bytes()
            {
                return InputRead::Ready(bytes.clone());
            }
            if source.len() > self.limits.file_bytes {
                return InputRead::Failed(InputError::TooLarge);
            }
            if state.bytes.saturating_add(source.len()) > self.limits.cached_bytes {
                return InputRead::Failed(InputError::CacheLimit);
            }
            drop(state);
            return self.cache(InputRead::Ready(Arc::from(source.as_bytes())));
        }
        match &root.reader {
            Ok(reader) => self.load(absolute, reader, relative),
            Err(error) => InputRead::Failed(error.clone()),
        }
    }
    // Callers resolve once and retain that path for decoding and diagnostics.
    fn source_bytes(&self, path: &Path) -> InputRead {
        if let Err(error) = self.request() {
            return InputRead::Failed(error);
        }
        if let Some(cached) = self.state.borrow().files.get(path) {
            return cached.read.clone();
        }
        let rooted = self.roots.iter().find_map(|root| {
            path.strip_prefix(&root.path)
                .ok()
                .map(|relative| (root.reader.clone(), relative.to_path_buf()))
        });
        let (reader, relative) = rooted.unwrap_or_else(|| {
            let parent = path.parent().unwrap();
            (
                self.resources.read_root(parent),
                PathBuf::from(path.file_name().unwrap()),
            )
        });
        match reader {
            Ok(reader) => self.load(path, &reader, &relative),
            // Compatibility for source-only third-party backends. Data reads
            // never take this fallback, even when the source is already cached.
            Err(InputError::Unavailable) => {
                let read = match self.resources.read(path) {
                    Ok(bytes) => InputRead::Ready(bytes.into()),
                    Err(ResourceError::NotFound(_)) => InputRead::Failed(InputError::NotFound),
                    Err(error) => InputRead::Failed(InputError::Io(error.to_string())),
                };
                self.store(path, read, None)
            }
            Err(error) => self.store(path, InputRead::Failed(error), None),
        }
    }
}
impl<R: Resources> EvaluationInputs for ResourceSession<R> {
    fn file(&self, path: &ResourcePath) -> InputRead {
        self.data(path)
    }
    fn data_cache(&self) -> Option<&dyn crate::ParsedDataCache> {
        Some(&self.data_cache)
    }
}
impl<R: Resources> Resources for ResourceSession<R> {
    fn root(&self) -> &Path {
        self.resources.root()
    }
    fn resolve(&self, path: &Path) -> PathBuf {
        self.resources.resolve(path)
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        let path = self.resolve(path);
        let read = self.source_bytes(&path);
        match read {
            InputRead::Ready(bytes) => Ok(bytes.to_vec()),
            InputRead::Failed(InputError::NotFound) => Err(ResourceError::NotFound(path)),
            InputRead::Failed(error) => Err(ResourceError::Access {
                path,
                message: error.to_string(),
            }),
            InputRead::Pending => Err(ResourceError::Access {
                path,
                message: "source input is not prepared".into(),
            }),
        }
    }
    fn source_shared(&self, path: &Path) -> Result<Arc<str>, ResourceError> {
        // Charge every logical request, including text-cache hits. This cache
        // never supplies data grants or substitutes registered document inputs.
        let path = self.resolve(path);
        let read = self.source_bytes(&path);
        let bytes = match read {
            InputRead::Ready(bytes) => bytes,
            InputRead::Failed(InputError::NotFound) => return Err(ResourceError::NotFound(path)),
            InputRead::Failed(error) => {
                return Err(ResourceError::Access {
                    path,
                    message: error.to_string(),
                });
            }
            InputRead::Pending => {
                return Err(ResourceError::Access {
                    path,
                    message: "source input is not prepared".into(),
                });
            }
        };
        if let Some(text) = self.state.borrow().texts.get(&path) {
            return text.clone();
        }
        let text = std::str::from_utf8(&bytes)
            .map(Arc::from)
            .map_err(|_| ResourceError::InvalidUtf8(path.clone()));
        // At most one decoded text per retained, bounded byte input.
        self.state.borrow_mut().texts.insert(path, text.clone());
        text
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        let path = self.resolve(path);
        if let Some(kind) = self.state.borrow().kinds.get(&path) {
            return kind.clone();
        }
        let kind = self.resources.kind(&path);
        self.state.borrow_mut().kinds.insert(path, kind.clone());
        kind
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        self.resources.entries(path)
    }
    fn read_root(&self, root: &Path) -> Result<Arc<dyn ResourceReader>, InputError> {
        self.resources.read_root(root)
    }
    fn source_root(&self, path: &Path) -> PathBuf {
        self.resources.source_root(path)
    }
}
