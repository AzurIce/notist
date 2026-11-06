//! Physical backing-store accounting, independent of selected logical values.
//! Holding a tiny child view must not hide a large retained document or text.
use super::*;
use crate::shared::{ArrayStorage, DictStorage};
use std::{collections::BTreeSet, sync::Weak};

#[derive(Default, Clone)]
pub struct DataRetention {
    documents: BTreeMap<usize, Data>,
    errors: BTreeMap<usize, DataError>,
    texts: BTreeSet<(usize, usize)>,
    arrays: BTreeMap<usize, Weak<ArrayStorage>>,
    dicts: BTreeMap<usize, Weak<DictStorage>>,
    bytes: usize,
}
impl fmt::Debug for DataRetention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataRetention")
            .field("bytes", &self.bytes)
            .field("documents", &self.documents.len())
            .field("errors", &self.errors.len())
            .finish()
    }
}
impl DataRetention {
    pub fn is_empty(&self) -> bool {
        self.bytes == 0
    }
    pub fn remaining(&self, limit: usize) -> usize {
        limit.saturating_sub(self.bytes)
    }
    fn charge(&mut self, bytes: usize, limit: usize) -> Result<(), DataFailure> {
        if bytes > self.remaining(limit) {
            return Err(DataFailure::Limit(
                "retained data backing-store limit exceeded",
            ));
        }
        self.bytes += bytes;
        Ok(())
    }
    pub fn observe(&mut self, value: &Value, limit: usize) -> Result<(), DataFailure> {
        if !value.contains_data_views() {
            return Ok(());
        }
        match value {
            Value::Data(data) => {
                let id = Arc::as_ptr(&data.document) as usize;
                if !self.documents.contains_key(&id) {
                    self.charge(data.document.resident_bytes.saturating_add(256), limit)?;
                    if let Some(text) = &data.document.text {
                        self.texts.insert((text.as_ptr() as usize, text.len()));
                    }
                    self.documents.insert(id, data.unbound());
                }
            }
            Value::DataError(error) => {
                let id = Arc::as_ptr(&error.detail) as usize;
                if !self.errors.contains_key(&id) {
                    let text = error.text.as_ref().map(|t| (t.as_ptr() as usize, t.len()));
                    let bytes = error.logical_size().saturating_add(384).saturating_add(
                        text.filter(|t| !self.texts.contains(t)).map_or(0, |t| t.1),
                    );
                    self.charge(bytes, limit)?;
                    if let Some(text) = text {
                        self.texts.insert(text);
                    }
                    self.errors.insert(
                        id,
                        DataError {
                            source: None,
                            ..error.clone()
                        },
                    );
                }
            }
            Value::Array(array) => {
                let (id, weak) = array.weak_identity();
                if self.arrays.contains_key(&id) {
                    return Ok(());
                }
                for value in array.iter() {
                    self.observe(value, limit)?;
                }
                self.charge(std::mem::size_of::<ArrayStorage>() + 128, limit)?;
                // A failed scan must not turn a later observation into a hit.
                self.arrays.insert(id, weak);
            }
            Value::Dict(dict) => {
                let (id, weak) = dict.weak_identity();
                if self.dicts.contains_key(&id) {
                    return Ok(());
                }
                for (_, value) in dict.iter() {
                    self.observe(value, limit)?;
                }
                self.charge(DictStorage::budget_bytes() + 128, limit)?;
                self.dicts.insert(id, weak);
            }
            Value::Some(value) | Value::Ok(value) | Value::Err(value) => {
                self.observe(value, limit)?
            }
            _ => {}
        }
        Ok(())
    }
}
