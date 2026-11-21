use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The first source submitted by the host for each document in this batch.
#[derive(Default)]
pub(super) struct DocumentInputs(HashMap<PathBuf, Arc<str>>);

impl DocumentInputs {
    pub(super) fn register(&mut self, path: &Path, source: Arc<str>) {
        if !self.0.contains_key(path) {
            self.0.insert(path.into(), source);
        }
    }

    pub(super) fn get(&self, path: &Path) -> Option<&str> {
        self.0.get(path).map(AsRef::as_ref)
    }
}
