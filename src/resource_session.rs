//! Host-owned input versions, data grants and shared bounded byte/error caches.
use crate::resources::{ResourceError, ResourceKind, ResourceReader, ResourceRevision};
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
}
struct Observation {
    reader: Arc<dyn ResourceReader>,
    relative: PathBuf,
    revision: Result<Option<ResourceRevision>, InputError>,
}
struct CachedInput {
    read: InputRead,
    observation: Option<Observation>,
}
#[derive(Default)]
struct State {
    files: BTreeMap<PathBuf, CachedInput>,
    kinds: BTreeMap<PathBuf, Result<Option<ResourceKind>, ResourceError>>,
    overlays: BTreeMap<PathBuf, Vec<Arc<[u8]>>>,
    sources: BTreeMap<PathBuf, Vec<Arc<str>>>,
    consumed: BTreeMap<PathBuf, InputRead>,
    dependencies: BTreeSet<ResourcePath>,
    bytes: usize,
    requests: usize,
    changed: bool,
}

/// One input batch. Construct a new session for a rebuild. Data access starts
/// denied; hosts explicitly grant logical roots independently of publication.
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
            self.roots.push(ReadRoot { path, reader });
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
    pub(crate) fn observe_source(&self, path: &Path, source: Arc<str>) {
        let mut state = self.state.borrow_mut();
        if state.consumed.get(path).is_some_and(
            |read| !matches!(read, InputRead::Ready(bytes) if bytes.as_ref() == source.as_bytes()),
        ) {
            state.changed = true;
        }
        let versions = state.sources.entry(path.into()).or_default();
        if !versions.iter().any(|text| text == &source) {
            versions.push(source);
        }
    }

    fn request(&self) -> Result<(), InputError> {
        let mut state = self.state.borrow_mut();
        if state.changed {
            return Err(InputError::Changed);
        }
        state.requests = state.requests.saturating_add(1);
        if state.requests > self.limits.requests {
            Err(InputError::RequestLimit)
        } else {
            Ok(())
        }
    }
    fn grant(&self, path: &Path) -> Result<(&ReadRoot, PathBuf), InputError> {
        let root = self
            .roots
            .iter()
            .find(|root| path.starts_with(&root.path))
            .ok_or(InputError::AccessDenied)?;
        Ok((root, path.strip_prefix(&root.path).unwrap().into()))
    }
    fn store(
        &self,
        path: &Path,
        mut read: InputRead,
        observation: Option<Observation>,
    ) -> InputRead {
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
        if matches!(read, InputRead::Failed(InputError::Changed)) {
            state.changed = true;
        }
        state.files.insert(
            path.into(),
            CachedInput {
                read: read.clone(),
                observation,
            },
        );
        read
    }
    fn load(&self, path: &Path, reader: Arc<dyn ResourceReader>, relative: PathBuf) -> InputRead {
        if let Some(cached) = self.state.borrow().files.get(path) {
            return cached.read.clone();
        }
        let before = reader.revision(&relative);
        let remaining = self
            .limits
            .cached_bytes
            .saturating_sub(self.state.borrow().bytes);
        let limit = self.limits.file_bytes.min(remaining);
        let mut read = reader.read_bounded(&relative, limit);
        if matches!(read, InputRead::Failed(InputError::TooLarge)) && limit < self.limits.file_bytes
        {
            read = InputRead::Failed(InputError::CacheLimit);
        }
        if !matches!(read, InputRead::Pending) && before != reader.revision(&relative) {
            read = InputRead::Failed(InputError::Changed);
        }
        let observation = (!matches!(read, InputRead::Pending)).then_some(Observation {
            reader,
            relative,
            revision: before,
        });
        self.store(path, read, observation)
    }
    /// Detect changes to every observed physical input, including missing files.
    /// This is not an atomic snapshot of the whole directory.
    pub fn validate(&self) -> Result<(), InputError> {
        let changed = {
            let state = self.state.borrow();
            state.changed
                || state
                    .files
                    .values()
                    .filter_map(|f| f.observation.as_ref())
                    .any(|o| o.reader.revision(&o.relative) != o.revision)
                || state
                    .kinds
                    .iter()
                    .any(|(path, kind)| self.resources.kind(path) != *kind)
        };
        if changed {
            self.state.borrow_mut().changed = true;
            Err(InputError::Changed)
        } else {
            Ok(())
        }
    }
    fn data(&self, path: &ResourcePath, source: Option<&str>) -> InputRead {
        if let Err(error) = self.request() {
            return InputRead::Failed(error);
        }
        self.state.borrow_mut().dependencies.insert(path.clone());
        let absolute = PathBuf::from(path.logical_path());
        let (root, relative) = match self.grant(&absolute) {
            Ok(grant) => grant,
            Err(error) => return InputRead::Failed(error),
        };
        let read = self.read_data(&absolute, root, relative, source);
        let mut state = self.state.borrow_mut();
        if matches!(read, InputRead::Failed(InputError::Changed))
            || state
                .consumed
                .get(&absolute)
                .is_some_and(|previous| previous != &read)
        {
            state.changed = true;
            return InputRead::Failed(InputError::Changed);
        }
        state
            .consumed
            .entry(absolute)
            .or_insert_with(|| read.clone());
        read
    }
    fn read_data(
        &self,
        absolute: &Path,
        root: &ReadRoot,
        relative: PathBuf,
        source: Option<&str>,
    ) -> InputRead {
        let observed = if source.is_none() {
            let state = self.state.borrow();
            match state.sources.get(absolute).map(Vec::as_slice) {
                Some([text]) => Some(text.clone()),
                // Several explicit revisions can coexist for editor analyses,
                // but an unrelated document cannot guess which one it means.
                Some(versions) if versions.len() > 1 => {
                    return InputRead::Failed(InputError::Changed);
                }
                _ => None,
            }
        } else {
            None
        };
        let source = source.or(observed.as_deref());
        // A caller-supplied source is an explicit per-document overlay. It need
        // not exist on disk, but an overlay cannot grant adjacent data access.
        if let Some(source) = source {
            let mut state = self.state.borrow_mut();
            if let Some(CachedInput {
                read: InputRead::Ready(bytes),
                ..
            }) = state.files.get(absolute)
                && bytes.as_ref() == source.as_bytes()
            {
                return InputRead::Ready(bytes.clone());
            }
            if let Some(bytes) = state.overlays.get(absolute).and_then(|versions| {
                versions
                    .iter()
                    .find(|bytes| bytes.as_ref() == source.as_bytes())
            }) {
                return InputRead::Ready(bytes.clone());
            }
            if source.len() > self.limits.file_bytes {
                return InputRead::Failed(InputError::TooLarge);
            }
            if state.bytes.saturating_add(source.len()) > self.limits.cached_bytes {
                return InputRead::Failed(InputError::CacheLimit);
            }
            let bytes = Arc::from(source.as_bytes());
            state.bytes += source.len();
            state
                .overlays
                .entry(absolute.to_path_buf())
                .or_default()
                .push(Arc::clone(&bytes));
            return InputRead::Ready(bytes);
        }
        let reader = match &root.reader {
            Ok(reader) => reader.clone(),
            Err(error) => return InputRead::Failed(error.clone()),
        };
        // Authorize through this root before consulting the shared source cache.
        // Capability-rooted filesystem probing cannot follow a link outside it.
        let revision = reader.revision(&relative);
        if let Err(error @ (InputError::AccessDenied | InputError::Unavailable)) = &revision {
            if !self.state.borrow().files.contains_key(absolute) {
                return self.store(
                    absolute,
                    InputRead::Failed(error.clone()),
                    Some(Observation {
                        reader,
                        relative,
                        revision,
                    }),
                );
            }
            return InputRead::Failed(error.clone());
        }
        if self
            .state
            .borrow()
            .files
            .get(absolute)
            .and_then(|f| f.observation.as_ref())
            .is_some_and(|o| o.revision != revision)
        {
            self.state.borrow_mut().changed = true;
            return InputRead::Failed(InputError::Changed);
        }
        if let Err(error) = self.validate() {
            return InputRead::Failed(error);
        }
        self.load(absolute, reader, relative)
    }
    /// Evaluation of explicit logical-path + text documents uses its exact text
    /// version, including when several analysis snapshots share a Vault.
    pub fn for_document<'a>(
        &'a self,
        path: &'a Path,
        source: &'a str,
    ) -> impl EvaluationInputs + 'a {
        DocumentInputs {
            session: self,
            path,
            source,
        }
    }
    fn source_bytes(&self, path: &Path) -> InputRead {
        let path = self.resources.resolve(path);
        if let Err(error) = self.request().and_then(|_| self.validate()) {
            return InputRead::Failed(error);
        }
        if let Some(cached) = self.state.borrow().files.get(&path) {
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
            Ok(reader) => self.load(&path, reader, relative),
            // Compatibility for source-only third-party backends. Data reads
            // never take this fallback, even when the source is already cached.
            Err(InputError::Unavailable) => {
                let read = match self.resources.read(&path) {
                    Ok(bytes) => InputRead::Ready(bytes.into()),
                    Err(ResourceError::NotFound(_)) => InputRead::Failed(InputError::NotFound),
                    Err(error) => InputRead::Failed(InputError::Io(error.to_string())),
                };
                self.store(&path, read, None)
            }
            Err(error) => self.store(&path, InputRead::Failed(error), None),
        }
    }
}
impl<R: Resources> EvaluationInputs for ResourceSession<R> {
    fn file(&self, path: &ResourcePath) -> InputRead {
        self.data(path, None)
    }
    fn data_cache(&self) -> Option<&dyn crate::ParsedDataCache> {
        Some(&self.data_cache)
    }
}
struct DocumentInputs<'a, R> {
    session: &'a ResourceSession<R>,
    path: &'a Path,
    source: &'a str,
}
impl<R: Resources> EvaluationInputs for DocumentInputs<'_, R> {
    fn file(&self, path: &ResourcePath) -> InputRead {
        self.session.data(
            path,
            (Path::new(&path.logical_path()) == self.path).then_some(self.source),
        )
    }
    fn data_cache(&self) -> Option<&dyn crate::ParsedDataCache> {
        Some(&self.session.data_cache)
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
        let read = self.source_bytes(path);
        match read {
            InputRead::Ready(bytes) => Ok(bytes.to_vec()),
            InputRead::Failed(InputError::NotFound) => {
                Err(ResourceError::NotFound(self.resolve(path)))
            }
            InputRead::Failed(error) => Err(ResourceError::Access {
                path: self.resolve(path),
                message: error.to_string(),
            }),
            InputRead::Pending => Err(ResourceError::Access {
                path: self.resolve(path),
                message: "source input is not prepared".into(),
            }),
        }
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
