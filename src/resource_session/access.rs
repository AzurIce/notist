//! Backend access and the batch-local file cache.
use crate::resources::{ResourceError, ResourceKind, ResourceReader};
use crate::{InputError, InputRead, Resources};
use std::{
    cell::RefCell,
    collections::HashMap,
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
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ResourceId(usize);

pub(super) struct DataAccess<'a> {
    pub(super) id: ResourceId,
    root: &'a ReadRoot,
    relative: &'a Path,
}
#[derive(Default)]
struct Resource {
    file: Option<InputRead>,
    kind: Option<Result<Option<ResourceKind>, ResourceError>>,
    text: Option<Result<Arc<str>, ResourceError>>,
}
#[derive(Default)]
struct State {
    identities: HashMap<PathBuf, ResourceId>,
    records: Vec<Resource>,
    bytes: usize,
    requests: usize,
}
impl State {
    fn identity(&mut self, path: &Path) -> ResourceId {
        if let Some(&id) = self.identities.get(path) {
            return id;
        }
        let id = ResourceId(self.records.len());
        self.records.push(Resource::default());
        self.identities.insert(path.into(), id);
        id
    }
}

pub(super) struct InputAccess<R> {
    resources: R,
    roots: Vec<ReadRoot>,
    limits: InputLimits,
    state: RefCell<State>,
}
impl<R: Resources> InputAccess<R> {
    pub(super) fn new(resources: R) -> Self {
        Self {
            resources,
            roots: Vec::new(),
            limits: InputLimits::default(),
            state: RefCell::default(),
        }
    }
    pub(super) fn resources(&self) -> &R {
        &self.resources
    }
    pub(super) fn set_limits(&mut self, limits: InputLimits) {
        self.limits = limits;
    }
    fn insert_root(&mut self, root: &Path) -> bool {
        let path = self.resources.resolve(root);
        if self.roots.iter().any(|r| r.path == path) {
            return false;
        }
        let reader = self.resources.read_root(&path);
        self.roots.push(ReadRoot { path, reader });
        self.roots
            .sort_by_key(|r| std::cmp::Reverse(r.path.components().count()));
        true
    }
    pub(super) fn add_root(&mut self, root: &Path) -> bool {
        let changed = self.insert_root(root);
        if changed {
            self.invalidate_reads();
        }
        changed
    }
    pub(super) fn replace_roots(&mut self, roots: impl IntoIterator<Item = PathBuf>) {
        self.roots.clear();
        for root in roots {
            self.insert_root(&root);
        }
        self.invalidate_reads();
    }
    fn invalidate_reads(&mut self) {
        let state = self.state.get_mut();
        for resource in &mut state.records {
            resource.file = None;
            resource.text = None;
        }
        state.bytes = 0;
    }
    pub(super) fn cached_bytes(&self) -> usize {
        self.state.borrow().bytes
    }

    pub(super) fn request(&self) -> Result<(), InputError> {
        let mut state = self.state.borrow_mut();
        state.requests = state.requests.saturating_add(1);
        if state.requests > self.limits.requests {
            Err(InputError::RequestLimit)
        } else {
            Ok(())
        }
    }
    fn authorize<'a>(&self, path: &'a Path) -> Result<(&ReadRoot, &'a Path), InputError> {
        let root = self
            .roots
            .iter()
            .find(|root| path.starts_with(&root.path))
            .ok_or(InputError::AccessDenied)?;
        Ok((root, path.strip_prefix(&root.path).unwrap()))
    }
    fn charge(&self, len: usize) -> Result<(), InputError> {
        if len > self.limits.file_bytes {
            return Err(InputError::TooLarge);
        }
        let mut state = self.state.borrow_mut();
        if len > self.limits.cached_bytes.saturating_sub(state.bytes) {
            return Err(InputError::CacheLimit);
        }
        state.bytes += len;
        Ok(())
    }
    fn retain_file(&self, id: ResourceId, read: InputRead) -> InputRead {
        let read = match &read {
            InputRead::Ready(bytes) => self
                .charge(bytes.len())
                .map_or_else(InputRead::Failed, |()| read),
            _ => read,
        };
        self.state.borrow_mut().records[id.0].file = Some(read.clone());
        read
    }
    fn fetch(&self, id: ResourceId, reader: &dyn ResourceReader, relative: &Path) -> InputRead {
        if let Some(read) = self.state.borrow().records[id.0].file.clone() {
            return read;
        }
        let remaining = {
            let state = self.state.borrow();
            self.limits.cached_bytes.saturating_sub(state.bytes)
        };
        let limit = self.limits.file_bytes.min(remaining);
        let mut read = reader.read_bounded(relative, limit);
        if matches!(read, InputRead::Failed(InputError::TooLarge)) && limit < self.limits.file_bytes
        {
            read = InputRead::Failed(InputError::CacheLimit);
        }
        self.retain_file(id, read)
    }
    pub(super) fn data_access<'a>(&'a self, path: &'a Path) -> Result<DataAccess<'a>, InputError> {
        let (root, relative) = self.authorize(path)?;
        let id = self.state.borrow_mut().identity(path);
        Ok(DataAccess { id, root, relative })
    }
    pub(super) fn read_data(&self, access: DataAccess<'_>) -> InputRead {
        // Check the reader before consulting the file cache: a source-only
        // backend may have filled the cache through the legacy fallback.
        match &access.root.reader {
            Ok(reader) => self.fetch(access.id, reader.as_ref(), access.relative),
            Err(error) => InputRead::Failed(error.clone()),
        }
    }
    pub(super) fn input_bytes(&self, id: ResourceId, text: &str) -> InputRead {
        if let Some(InputRead::Ready(bytes)) = &self.state.borrow().records[id.0].file
            && bytes.as_ref() == text.as_bytes()
        {
            return InputRead::Ready(bytes.clone());
        }
        match self.charge(text.len()) {
            Ok(()) => InputRead::Ready(Arc::from(text.as_bytes())),
            Err(error) => InputRead::Failed(error),
        }
    }
    // Callers resolve once and retain that path for decoding and diagnostics.
    fn source_bytes(&self, path: &Path) -> Result<(ResourceId, InputRead), InputError> {
        self.request()?;
        let id = self.state.borrow_mut().identity(path);
        if let Some(cached) = &self.state.borrow().records[id.0].file {
            return Ok((id, cached.clone()));
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
        let read = match reader {
            Ok(reader) => self.fetch(id, reader.as_ref(), &relative),
            // Compatibility for source-only third-party backends. Data reads
            // never take this fallback, even when the source is already cached.
            Err(InputError::Unavailable) => {
                let read = match self.resources.read(path) {
                    Ok(bytes) => InputRead::Ready(bytes.into()),
                    Err(ResourceError::NotFound(_)) => InputRead::Failed(InputError::NotFound),
                    Err(error) => InputRead::Failed(InputError::Io(error.to_string())),
                };
                self.retain_file(id, read)
            }
            Err(error) => self.retain_file(id, InputRead::Failed(error)),
        };
        Ok((id, read))
    }
}
impl<R: Resources> Resources for InputAccess<R> {
    fn root(&self) -> &Path {
        self.resources.root()
    }
    fn resolve(&self, path: &Path) -> PathBuf {
        self.resources.resolve(path)
    }
    fn read(&self, path: &Path) -> Result<Vec<u8>, ResourceError> {
        let path = self.resolve(path);
        let read = self
            .source_bytes(&path)
            .map(|(_, read)| read)
            .unwrap_or_else(InputRead::Failed);
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
        let (id, read) = self
            .source_bytes(&path)
            .map_err(|error| ResourceError::Access {
                path: path.clone(),
                message: error.to_string(),
            })?;
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
        if let Some(text) = &self.state.borrow().records[id.0].text {
            return text.clone();
        }
        let text = std::str::from_utf8(&bytes)
            .map(Arc::from)
            .map_err(|_| ResourceError::InvalidUtf8(path.clone()));
        // At most one decoded text per retained, bounded byte input.
        self.state.borrow_mut().records[id.0].text = Some(text.clone());
        text
    }
    fn kind(&self, path: &Path) -> Result<Option<ResourceKind>, ResourceError> {
        let path = self.resolve(path);
        let id = self.state.borrow_mut().identity(&path);
        if let Some(kind) = &self.state.borrow().records[id.0].kind {
            return kind.clone();
        }
        let kind = self.resources.kind(&path);
        self.state.borrow_mut().records[id.0].kind = Some(kind.clone());
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

#[cfg(test)]
mod identity_tests {
    use super::*;
    use crate::MemoryResources;

    #[test]
    fn file_cache_shares_one_identity_without_granting_access() {
        let mut files = MemoryResources::new("/project");
        files.insert("item.txt", "bytes");
        let session = InputAccess::new(files);
        assert_eq!(
            session.kind(Path::new("item.txt")).unwrap(),
            Some(ResourceKind::File)
        );
        assert!(session.state.borrow().records[0].file.is_none());
        assert_eq!(
            session
                .source_shared(Path::new("./item.txt"))
                .unwrap()
                .as_ref(),
            "bytes"
        );
        assert_eq!(session.state.borrow().records.len(), 1);
        assert_eq!(session.state.borrow().identities.len(), 1);
        assert!(matches!(
            session.data_access(Path::new("/project/item.txt")),
            Err(InputError::AccessDenied)
        ));
        let state = session.state.borrow();
        assert!(state.records[0].kind.is_some());
        assert!(state.records[0].file.is_some());
        assert!(state.records[0].text.is_some());
    }

    #[test]
    fn exhausted_requests_do_not_intern_more_paths() {
        let mut files = MemoryResources::new("/project");
        files.insert("first", "one");
        let mut session = InputAccess::new(files);
        session.set_limits(InputLimits {
            requests: 1,
            ..Default::default()
        });
        session.read(Path::new("first")).unwrap();
        for i in 0..100 {
            assert!(session.read(Path::new(&format!("other-{i}"))).is_err());
        }
        assert_eq!(session.state.borrow().records.len(), 1);
    }
}
