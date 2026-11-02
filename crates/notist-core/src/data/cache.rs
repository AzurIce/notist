use super::*;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

pub trait ParsedDataCache {
    /// Called only after file() authorization/version/dependency accounting.
    fn parse(
        &self,
        format: DataFormat,
        bytes: Arc<[u8]>,
        limits: DataLimits,
    ) -> Result<Data, DataFailure>;
    fn decode(&self, data: &Data, target: &Type, limits: DataLimits) -> Result<Value, DataFailure>;
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DataCacheStats {
    pub parses: usize,
    pub projections: usize,
    pub bytes: usize,
    pub entries: usize,
}
type ParseKey = (DataFormat, DataLimits, usize, usize);
type ProjectionKey = (usize, usize, Type, DataLimits);
type ParseEntry = (Arc<[u8]>, Result<Data, DataFailure>);
type ProjectionEntry = (Data, Result<Value, DataFailure>);
#[derive(Default)]
struct State {
    // Retained Arc bytes establish non-reusable immutable input identity; no
    // pointer is persisted, exposed, or used for semantic equality.
    parsed: BTreeMap<ParseKey, ParseEntry>,
    projected: BTreeMap<ProjectionKey, ProjectionEntry>,
    documents: BTreeSet<usize>,
    stats: DataCacheStats,
}
pub struct DataCache {
    state: RefCell<State>,
    max_bytes: usize,
    max_entries: usize,
}
impl Default for DataCache {
    fn default() -> Self {
        Self::new(128 * 1024 * 1024, 4096)
    }
}
impl DataCache {
    pub fn new(max_bytes: usize, max_entries: usize) -> Self {
        Self {
            state: RefCell::default(),
            max_bytes,
            max_entries,
        }
    }
    pub fn stats(&self) -> DataCacheStats {
        self.state.borrow().stats
    }
    fn reserve(&self, state: &mut State, bytes: usize) -> Result<(), DataFailure> {
        if state.stats.entries >= self.max_entries
            || state.stats.bytes.saturating_add(bytes) > self.max_bytes
        {
            return Err(DataFailure::Limit("data cache limit exceeded"));
        }
        state.stats.bytes += bytes;
        state.stats.entries += 1;
        Ok(())
    }
}
impl ParsedDataCache for DataCache {
    fn parse(
        &self,
        format: DataFormat,
        bytes: Arc<[u8]>,
        limits: DataLimits,
    ) -> Result<Data, DataFailure> {
        let key = (format, limits, bytes.as_ptr() as usize, bytes.len());
        let mut state = self.state.borrow_mut();
        if let Some((_, result)) = state.parsed.get(&key) {
            return result.clone();
        }
        if state.stats.entries >= self.max_entries {
            return Err(DataFailure::Limit("data cache limit exceeded"));
        }
        // Bound peak parser/tree allocation independently of committed cache
        // entries. Format DOM overhead is bounded by source + parser limits.
        if state.stats.bytes.saturating_add(
            bytes
                .len()
                .saturating_mul(if format == DataFormat::Toml { 64 } else { 8 }),
        ) > self.max_bytes
        {
            return Err(DataFailure::Limit(
                "data cache parse working-set limit exceeded",
            ));
        }
        state.stats.parses += 1;
        let mut effective = limits;
        effective.working_bytes = effective.working_bytes.min(
            self.max_bytes
                .saturating_sub(state.stats.bytes)
                .saturating_sub(bytes.len()),
        );
        let result = super::parse(format, &bytes, effective);
        let size = bytes.len().saturating_add(match &result {
            Ok(data) => data.document.resident_bytes,
            Err(DataFailure::Error(e)) => e.resident_bytes(),
            Err(DataFailure::Limit(_)) => 64,
        });
        self.reserve(&mut state, size.saturating_add(256))?;
        if let Ok(data) = &result {
            state.documents.insert(Arc::as_ptr(&data.document) as usize);
        }
        state.parsed.insert(key, (bytes, result.clone()));
        result
    }
    fn decode(&self, data: &Data, target: &Type, limits: DataLimits) -> Result<Value, DataFailure> {
        // Data targets must return the caller's exact view, never a cached
        // source from another document or an owning cycle inside Document.
        if *target == Type::Data {
            return data.decode(target, limits);
        }
        let key = (
            Arc::as_ptr(&data.document) as usize,
            data.node,
            target.clone(),
            limits,
        );
        let result = {
            let mut state = self.state.borrow_mut();
            if let Some((_, result)) = state.projected.get(&key) {
                result.clone()
            } else {
                if state.stats.entries >= self.max_entries {
                    return Err(DataFailure::Limit("data cache limit exceeded"));
                }
                state.stats.projections += 1;
                let unbound = data.unbound();
                let result = unbound.decode(target, limits);
                let size = match &result {
                    Ok(v) => v.resident_size(),
                    Err(DataFailure::Error(e)) => e.logical_size(),
                    Err(DataFailure::Limit(_)) => 64,
                };
                let document = if state.documents.contains(&key.0) {
                    0
                } else {
                    data.document.resident_bytes
                };
                self.reserve(
                    &mut state,
                    size.saturating_add(8192).saturating_add(document),
                )?;
                state.documents.insert(key.0);
                state.projected.insert(key, (unbound, result.clone()));
                result
            }
        };
        match result {
            Ok(value) => Ok(bind_value(&value, &data.source)),
            Err(DataFailure::Error(mut error)) => {
                error.source = data.source.clone();
                Err(error.into())
            }
            Err(limit) => Err(limit),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropping_the_input_batch_releases_documents_even_for_data_projections() {
        let cache = DataCache::default();
        let limits = DataLimits::default();
        let root = cache
            .parse(DataFormat::Json, Arc::from(b"[1,2,3]".as_slice()), limits)
            .unwrap();
        let weak = Arc::downgrade(&root.document);
        let value = cache
            .decode(&root, &Type::Array(Some(Box::new(Type::Data))), limits)
            .unwrap();
        drop(root);
        drop(value);
        drop(cache);
        assert!(weak.upgrade().is_none());
    }
}
