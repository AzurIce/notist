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
    /// The evaluator bypasses this callback for [`direct_decode_target`] types.
    fn decode(&self, data: &Data, target: &Type, limits: DataLimits) -> Result<Value, DataFailure>;
}

/// Targets decoded directly before consulting a host's shared projection cache.
/// Data must preserve the caller's exact source-bound view without retaining an
/// owning cycle inside Document. Scalars (including nested Options) share their
/// backing strings and must not retain one memo entry per record.
pub fn direct_decode_target(target: &Type) -> bool {
    match target {
        Type::Data | Type::Unit | Type::Bool | Type::Int | Type::Float | Type::String => true,
        Type::Option(element) => direct_decode_target(element),
        _ => false,
    }
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
        if direct_decode_target(target) {
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
    fn direct_targets_never_borrow_or_reserve_cache_state() {
        let cache = DataCache::new(0, 0);
        let limits = DataLimits::default();
        // Holding the state borrow makes any accidental cache access fail.
        let state = cache.state.borrow_mut();
        for (text, target) in [
            ("{\"x\":1}", Type::Data),
            ("null", Type::Unit),
            ("true", Type::Bool),
            ("7", Type::Int),
            ("1.5", Type::Float),
            ("\"shared\"", Type::String),
        ] {
            let data = super::super::parse(DataFormat::Json, text.as_bytes(), limits).unwrap();
            let mut target = target;
            for _ in 0..3 {
                assert_eq!(
                    cache.decode(&data, &target, limits),
                    data.decode(&target, limits)
                );
                for strict in [
                    DataLimits { nodes: 0, ..limits },
                    DataLimits { depth: 0, ..limits },
                    DataLimits {
                        value_bytes: 0,
                        ..limits
                    },
                ] {
                    let result = cache.decode(&data, &target, strict);
                    assert!(matches!(result, Err(DataFailure::Limit(_))));
                    assert_eq!(result, data.decode(&target, strict));
                }
                // Plain views/scalars have no owned resident storage; strings
                // and non-null Options do. Preserve both working-set outcomes.
                let strict = DataLimits {
                    working_bytes: 0,
                    ..limits
                };
                assert_eq!(
                    cache.decode(&data, &target, strict),
                    data.decode(&target, strict)
                );
                target = Type::Option(Box::new(target));
            }
        }
        let data = super::super::parse(DataFormat::Json, b"7", limits).unwrap();
        assert!(matches!(
            cache.decode(&data, &Type::String, limits),
            Err(DataFailure::Error(error)) if error.kind() == DataErrorKind::TypeMismatch
        ));
        assert_eq!(state.stats, DataCacheStats::default());
    }

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
