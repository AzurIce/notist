//! Compose backend access with the host's document inputs for evaluation.
mod access;
mod documents;

pub use access::InputLimits;
use access::{InputAccess, ResourceId};
use documents::DocumentInputs;

use crate::resources::{ResourceError, ResourceKind, ResourceReader};
use crate::{
    DataCache, DataCacheStats, EvaluationInputs, InputError, InputRead, ParsedDataCache,
    ResourcePath, Resources,
};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::Arc,
};

/// A lazy input batch, not an atomic filesystem snapshot. Changing the read
/// roots starts a new read view while keeping the host's document submissions.
pub struct ResourceSession<R> {
    access: InputAccess<R>,
    documents: DocumentInputs,
    // Freeze evaluation answers independently of backend reads: a host edit
    // wins over a cached file, but never replaces an answer already returned.
    answers: RefCell<HashMap<ResourceId, InputRead>>,
    dependencies: RefCell<BTreeSet<ResourcePath>>,
    data_cache: DataCache,
}

impl<R: Resources> ResourceSession<R> {
    pub fn new(resources: R) -> Self {
        Self {
            access: InputAccess::new(resources),
            documents: Default::default(),
            answers: Default::default(),
            dependencies: Default::default(),
            data_cache: Default::default(),
        }
    }

    pub fn resources(&self) -> &R {
        self.access.resources()
    }

    pub fn with_limits(mut self, limits: InputLimits) -> Self {
        self.access.set_limits(limits);
        self
    }

    pub fn with_read_root(mut self, root: impl AsRef<Path>) -> Self {
        if self.access.add_root(root.as_ref()) {
            self.clear_read_view();
        }
        self
    }

    pub fn with_read_roots(mut self, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        self.access.replace_roots(roots);
        self.clear_read_view();
        self
    }

    fn clear_read_view(&mut self) {
        self.answers.get_mut().clear();
        self.dependencies.get_mut().clear();
    }

    pub fn dependencies(&self) -> BTreeSet<ResourcePath> {
        self.dependencies.borrow().clone()
    }

    pub fn cached_bytes(&self) -> usize {
        self.access.cached_bytes()
    }

    pub fn with_data_cache_limits(mut self, bytes: usize, entries: usize) -> Self {
        self.data_cache = DataCache::new(bytes, entries);
        self
    }

    pub fn data_cache_stats(&self) -> DataCacheStats {
        self.data_cache.stats()
    }

    pub(crate) fn register_source(&mut self, path: &Path, source: Arc<str>) {
        self.documents.register(path, source);
    }
}

impl<R: Resources> EvaluationInputs for ResourceSession<R> {
    fn file(&self, path: &ResourcePath) -> InputRead {
        if let Err(error) = self.access.request() {
            return InputRead::Failed(error);
        }
        self.dependencies.borrow_mut().insert(path.clone());
        let absolute = PathBuf::from(path.logical_path());
        let access = match self.access.data_access(&absolute) {
            Ok(access) => access,
            Err(error) => return InputRead::Failed(error),
        };
        let id = access.id;
        if let Some(read) = self.answers.borrow().get(&id) {
            return read.clone();
        }
        let read = match self.documents.get(&absolute) {
            Some(text) => self.access.input_bytes(id, text),
            None => self.access.read_data(access),
        };
        self.answers.borrow_mut().insert(id, read.clone());
        read
    }

    fn data_cache(&self) -> Option<&dyn ParsedDataCache> {
        Some(&self.data_cache)
    }
}

impl<R: Resources> Resources for ResourceSession<R> {
    fn root(&self) -> &Path {
        self.access.root()
    }
    fn resolve(&self, path: &Path) -> PathBuf {
        self.access.resolve(path)
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        self.access.read(path)
    }
    fn source_shared(&self, path: &Path) -> Result<Arc<str>, ResourceError> {
        self.access.source_shared(path)
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        self.access.kind(path)
    }
    fn entries(&self, path: &Path) -> Result<Vec<PathBuf>, ResourceError> {
        self.access.entries(path)
    }
    fn read_root(&self, root: &Path) -> Result<Arc<dyn ResourceReader>, InputError> {
        self.access.read_root(root)
    }
    fn source_root(&self, path: &Path) -> PathBuf {
        self.access.source_root(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemoryResources;

    #[test]
    fn virtual_document_is_a_data_input_not_a_backend_file() {
        let mut session =
            ResourceSession::new(MemoryResources::new("/project")).with_read_root("/project");
        session.register_source(Path::new("/project/virtual.not"), Arc::from("virtual"));
        let path = ResourcePath::resolve("/project", "doc.not", "virtual.not").unwrap();
        assert_eq!(
            session.file(&path),
            InputRead::Ready(Arc::from(&b"virtual"[..]))
        );
        assert!(matches!(
            session.source_shared(Path::new("virtual.not")),
            Err(ResourceError::NotFound(_))
        ));
    }

    #[test]
    fn document_inputs_survive_read_view_changes_without_bypassing_grants() {
        let mut files = MemoryResources::new("/project");
        files.insert("page.not", "disk");
        let mut session = ResourceSession::new(files).with_read_root("/project");
        let path = ResourcePath::resolve("/project", "doc.not", "page.not").unwrap();
        assert_eq!(
            session.file(&path),
            InputRead::Ready(Arc::from(&b"disk"[..]))
        );
        session.register_source(Path::new("/project/page.not"), Arc::from("edited"));
        assert_eq!(
            session.file(&path),
            InputRead::Ready(Arc::from(&b"disk"[..]))
        );

        let session = session.with_read_roots([]);
        assert_eq!(
            session.file(&path),
            InputRead::Failed(InputError::AccessDenied)
        );
        let session = session.with_read_root("/project");
        assert_eq!(
            session.file(&path),
            InputRead::Ready(Arc::from(&b"edited"[..]))
        );
        assert_eq!(
            session
                .source_shared(Path::new("page.not"))
                .unwrap()
                .as_ref(),
            "disk"
        );
    }
}
