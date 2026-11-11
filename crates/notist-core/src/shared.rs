//! Shared immutable language containers. Builders use copy-on-write; wire data
//! and equality retain the previous string/vector/ordered-entry semantics.
use crate::{definitions::Type, item::Value};
use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    fmt,
    ops::{Deref, DerefMut},
    sync::{Arc, Mutex, OnceLock},
};

/// An owned value whose clones share immutable storage. Mutable access detaches
/// only when another owner still observes the previous value.
///
/// This is ownership sharing, not language-level identity or result memoization.
/// Equality, debugging and serialization expose the contained value, not its Arc.
pub struct Shared<T>(Arc<T>);
impl<T> Shared<T> {
    pub fn new(value: T) -> Self {
        Self(Arc::new(value))
    }
    pub fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl<T: Clone> Shared<T> {
    /// Consume unique storage without cloning, or copy a still-shared value.
    pub fn into_inner(self) -> T {
        Arc::unwrap_or_clone(self.0)
    }
}
impl<T> Clone for Shared<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T: Default> Default for Shared<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}
impl<T> From<T> for Shared<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}
impl<T> From<Box<T>> for Shared<T> {
    fn from(value: Box<T>) -> Self {
        Self(Arc::from(value))
    }
}
impl<T> Deref for Shared<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}
impl<T: Clone> DerefMut for Shared<T> {
    fn deref_mut(&mut self) -> &mut T {
        Arc::make_mut(&mut self.0)
    }
}
impl<T> AsRef<T> for Shared<T> {
    fn as_ref(&self) -> &T {
        self
    }
}
impl<T: fmt::Debug> fmt::Debug for Shared<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}
impl<T: PartialEq> PartialEq for Shared<T> {
    fn eq(&self, other: &Self) -> bool {
        // Do not infer value equality from storage identity (notably for NaN).
        self.as_ref().eq(other.as_ref())
    }
}
impl<T: Eq> Eq for Shared<T> {}

impl<T> FromIterator<T> for Shared<Vec<T>> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::new(iter.into_iter().collect())
    }
}

#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Text(Arc<str>);
impl Text {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl From<String> for Text {
    fn from(s: String) -> Self {
        Self(Arc::from(s))
    }
}
impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}
impl From<Arc<str>> for Text {
    fn from(s: Arc<str>) -> Self {
        Self(s)
    }
}
impl From<Text> for String {
    fn from(s: Text) -> Self {
        s.to_string()
    }
}
impl Deref for Text {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<str> for Text {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl Borrow<str> for Text {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}
impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}
impl fmt::Display for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self)
    }
}
impl PartialEq<str> for Text {
    fn eq(&self, s: &str) -> bool {
        self.as_str() == s
    }
}
impl PartialEq<&str> for Text {
    fn eq(&self, s: &&str) -> bool {
        self.as_str() == *s
    }
}
impl PartialEq<String> for Text {
    fn eq(&self, s: &String) -> bool {
        self.as_str() == s
    }
}
impl PartialEq<Text> for str {
    fn eq(&self, s: &Text) -> bool {
        self == s.as_str()
    }
}
impl PartialEq<Text> for String {
    fn eq(&self, s: &Text) -> bool {
        self == s.as_str()
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Metrics {
    pub bytes: usize,
    pub resident: usize,
    pub path: bool,
    pub data: bool,
}
impl Metrics {
    fn add(&mut self, other: Self) {
        self.bytes = self.bytes.saturating_add(other.bytes);
        self.resident = self.resident.saturating_add(other.resident);
        self.path |= other.path;
        self.data |= other.data;
    }
}

#[derive(Clone, Default)]
pub struct Array(Arc<ArrayStorage>);
#[derive(Default)]
pub(crate) struct ArrayStorage {
    values: Vec<Value>,
    metrics: OnceLock<Metrics>,
    inferred: OnceLock<Type>,
    validated: Mutex<Validation>,
}
#[derive(Default)]
struct Validation {
    types: BTreeSet<Type>,
    bytes: usize,
}
const VALIDATION_BYTES: usize = 4096;
const INFERRED_BYTES: usize = 512;
fn type_cost(ty: &Type, limit: usize) -> usize {
    fn cost(ty: &Type, left: &mut usize, depth: usize) -> bool {
        let size = std::mem::size_of::<Type>() + 64;
        if depth == 0 || *left < size {
            return false;
        }
        *left -= size;
        match ty {
            Type::Array(Some(t)) | Type::Option(t) => cost(t, left, depth - 1),
            Type::Result(a, b) => cost(a, left, depth - 1) && cost(b, left, depth - 1),
            Type::Parameter(name) => {
                if name.len() > *left {
                    false
                } else {
                    *left -= name.len();
                    true
                }
            }
            _ => true,
        }
    }
    let mut left = limit;
    if cost(ty, &mut left, 128) {
        limit - left
    } else {
        limit.saturating_add(1)
    }
}
impl Array {
    pub(crate) fn weak_identity(&self) -> (usize, std::sync::Weak<ArrayStorage>) {
        (Arc::as_ptr(&self.0) as usize, Arc::downgrade(&self.0))
    }
    pub fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(crate) fn metrics(&self) -> Metrics {
        *self.0.metrics.get_or_init(|| {
            let mut result = Metrics {
                bytes: self.len().saturating_mul(8),
                resident: self
                    .0
                    .values
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Value>())
                    .saturating_add(std::mem::size_of::<ArrayStorage>() + 16 + INFERRED_BYTES)
                    .saturating_add(if self.is_empty() { 0 } else { VALIDATION_BYTES }),
                ..Default::default()
            };
            for value in self.iter() {
                result.add(value.metrics());
            }
            result
        })
    }
    pub(crate) fn accepts(&self, ty: &Type) -> bool {
        if self.is_empty() || self.0.validated.lock().unwrap().types.contains(ty) {
            return true;
        }
        if !self.iter().all(|v| ty.accepts(v)) {
            return false;
        }
        let mut cache = self.0.validated.lock().unwrap();
        let cost = type_cost(ty, VALIDATION_BYTES);
        if cache.types.len() < 64
            && cache.bytes.saturating_add(cost) <= VALIDATION_BYTES
            && !cache.types.contains(ty)
        {
            cache.bytes += cost;
            cache.types.insert(ty.clone());
        }
        true
    }
    pub(crate) fn inferred_type(&self) -> Type {
        if let Some(ty) = self.0.inferred.get() {
            return ty.clone();
        }
        let first = self.first().map(Type::of);
        let ty = Type::Array(
            first
                .filter(|t| self.iter().all(|v| Type::of(v) == *t))
                .map(Box::new),
        );
        if type_cost(&ty, INFERRED_BYTES) <= INFERRED_BYTES {
            let _ = self.0.inferred.set(ty.clone());
        }
        ty
    }
}
impl Deref for Array {
    type Target = [Value];
    fn deref(&self) -> &[Value] {
        &self.0.values
    }
}
impl From<Vec<Value>> for Array {
    fn from(values: Vec<Value>) -> Self {
        Self(Arc::new(ArrayStorage {
            values,
            ..Default::default()
        }))
    }
}
impl FromIterator<Value> for Array {
    fn from_iter<T: IntoIterator<Item = Value>>(iter: T) -> Self {
        Vec::from_iter(iter).into()
    }
}
impl IntoIterator for Array {
    type Item = Value;
    type IntoIter = std::vec::IntoIter<Value>;
    fn into_iter(self) -> Self::IntoIter {
        Arc::try_unwrap(self.0)
            .map(|v| v.values)
            .unwrap_or_else(|v| v.values.clone())
            .into_iter()
    }
}
impl<'a> IntoIterator for &'a Array {
    type Item = &'a Value;
    type IntoIter = std::slice::Iter<'a, Value>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}
impl fmt::Debug for Array {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.values.fmt(f)
    }
}
impl PartialEq for Array {
    fn eq(&self, other: &Self) -> bool {
        self.0.values == other.0.values
    }
}

/// Ordered fields with copy-on-write storage; capacity-free empties allocate
/// nothing while retaining the conservative resident budget of an owned Dict.
#[derive(Clone, Default)]
pub struct Dict(Option<Arc<DictStorage>>);

/// Field operations shared by semantic dictionaries and their owned construction phase.
pub trait Fields {
    fn get(&self, key: &str) -> Option<&Value>;
    fn insert(&mut self, key: &str, value: Value);
}

/// Unique storage until the final dictionary escapes into a semantic value.
#[derive(Debug, Default)]
pub struct DictBuilder {
    entries: Vec<(Text, Value)>,
    budget_capacity: usize,
}
impl DictBuilder {
    pub fn insert(&mut self, key: Text, value: Value) {
        if let Some((_, previous)) = self.entries.iter_mut().find(|(name, _)| *name == key) {
            *previous = value;
        } else {
            if self.entries.len() == self.budget_capacity {
                self.budget_capacity = self.budget_capacity.saturating_mul(2).max(4);
            }
            if self.entries.capacity() == 0 {
                self.entries.reserve_exact(1);
            }
            self.entries.push((key, value));
        }
    }
    pub fn finish(self) -> Dict {
        Dict::from_entries(self.entries, self.budget_capacity)
    }
}
impl Fields for DictBuilder {
    fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .find(|(name, _)| name.as_str() == key)
            .map(|(_, v)| v)
    }
    fn insert(&mut self, key: &str, value: Value) {
        self.insert(key.into(), value);
    }
}
impl Fields for Dict {
    fn get(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
    fn insert(&mut self, key: &str, value: Value) {
        self.insert(key, value);
    }
}
#[derive(Default)]
pub(crate) struct DictStorage {
    entries: Vec<(Text, Value)>,
    index: Option<Box<BTreeMap<Text, usize>>>,
    budget_capacity: usize,
    metrics: OnceLock<Metrics>,
}

// Keep the previous conservative accounting layout even when the physical
// representation changes. The wide tree is boxed only to make room for the
// capacity shadow without increasing the small-dictionary storage size.
#[allow(dead_code)]
struct DictBudgetLayout {
    entries: Vec<(Text, Value)>,
    index: BTreeMap<Text, usize>,
    metrics: OnceLock<Metrics>,
}

impl Clone for DictStorage {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
            index: self.index.clone(),
            // Vec::clone in the old representation allocated exactly len slots.
            // Cached metrics are cloned too, as before, and insertion clears them.
            budget_capacity: self.entries.len(),
            metrics: self.metrics.clone(),
        }
    }
}

// Tiny dictionaries dominate content fields/annotations. Their ordered entries
// are already a cheaper lookup table than an independently allocated tree leaf.
const DICT_LINEAR_LIMIT: usize = 8;

impl DictStorage {
    pub(crate) fn budget_bytes() -> usize {
        std::mem::size_of::<DictBudgetLayout>()
    }

    fn position(&self, key: &str) -> Option<usize> {
        if self.entries.len() <= DICT_LINEAR_LIMIT {
            self.entries
                .iter()
                .position(|(name, _)| name.as_str() == key)
        } else {
            self.index
                .as_ref()
                .and_then(|index| index.get(key))
                .copied()
        }
    }

    fn make_index(entries: &[(Text, Value)]) -> Option<Box<BTreeMap<Text, usize>>> {
        let mut index = BTreeMap::new();
        if entries.len() > DICT_LINEAR_LIMIT {
            for (i, (key, _)) in entries.iter().enumerate() {
                // Deserialized dictionaries can retain duplicate entries; lookup
                // and mutation must continue selecting the first occurrence.
                index.entry(key.clone()).or_insert(i);
            }
            Some(Box::new(index))
        } else {
            None
        }
    }

    fn index_entries_for_budget(&self) -> usize {
        if self.entries.len() > DICT_LINEAR_LIMIT {
            self.index.as_ref().map_or(0, |index| index.len())
        } else {
            // Keep the previous conservative budget proxy even though the
            // physical index is now absent. Do not make working limits looser.
            self.entries
                .iter()
                .enumerate()
                .filter(|(i, (key, _))| {
                    !self.entries[..*i]
                        .iter()
                        .any(|(previous, _)| previous == key)
                })
                .count()
        }
    }
}

impl Dict {
    fn from_entries(entries: Vec<(Text, Value)>, budget_capacity: usize) -> Self {
        if entries.is_empty() && budget_capacity == 0 {
            return Self::default();
        }
        let index = DictStorage::make_index(&entries);
        Self(Some(Arc::new(DictStorage {
            entries,
            index,
            budget_capacity,
            metrics: OnceLock::new(),
        })))
    }
    fn storage(&self) -> &DictStorage {
        // Empty dictionaries need neither an allocation nor a shared Arc
        // refcount. Accounting still charges the previous conservative layout.
        static EMPTY: DictStorage = DictStorage {
            entries: Vec::new(),
            index: None,
            budget_capacity: 0,
            metrics: OnceLock::new(),
        };
        self.0.as_deref().unwrap_or(&EMPTY)
    }
    pub(crate) fn weak_identity(&self) -> (usize, std::sync::Weak<DictStorage>) {
        // Storage-free empties cannot contain Data and are never entered in
        // DataRetention's weak-identity table. Do not make an immortal weak Arc.
        self.0.as_ref().map_or_else(
            || (0, std::sync::Weak::new()),
            |storage| (Arc::as_ptr(storage) as usize, Arc::downgrade(storage)),
        )
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        let data = self.storage();
        data.position(key).map(|i| &data.entries[i].1)
    }
    pub fn is_empty(&self) -> bool {
        self.storage().entries.is_empty()
    }
    pub fn insert(&mut self, key: impl AsRef<str>, value: Value) {
        let key = key.as_ref();
        let data = Arc::make_mut(
            self.0
                .get_or_insert_with(|| Arc::new(DictStorage::default())),
        );
        data.metrics = OnceLock::new();
        match data.position(key) {
            Some(i) => data.entries[i].1 = value,
            None => {
                let key: Text = key.into();
                let was_linear = data.entries.len() == DICT_LINEAR_LIMIT;
                if data.entries.len() == data.budget_capacity {
                    // The former Vec<(Text, Value)> grows to at least four
                    // entries, then doubles. Preserve that budget independently
                    // of the physically compact first allocation.
                    data.budget_capacity = data.budget_capacity.saturating_mul(2).max(4);
                }
                if data.entries.capacity() == 0 {
                    data.entries.reserve_exact(1);
                }
                data.entries.push((key, value));
                if was_linear {
                    data.index = DictStorage::make_index(&data.entries);
                } else if data.entries.len() > DICT_LINEAR_LIMIT {
                    let i = data.entries.len() - 1;
                    data.index
                        .as_mut()
                        .expect("wide dictionary index")
                        .insert(data.entries[i].0.clone(), i);
                }
            }
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.storage().entries.iter().map(|(k, v)| (k.as_str(), v))
    }
    pub fn extend(&mut self, other: Dict) {
        for (k, v) in other.iter() {
            self.insert(k, v.clone());
        }
    }
    pub fn take(&mut self) -> Dict {
        std::mem::take(self)
    }
    pub fn shares_storage(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
    pub(crate) fn metrics(&self) -> Metrics {
        let data = self.storage();
        *data.metrics.get_or_init(|| {
            let mut result = Metrics {
                resident: data
                    .budget_capacity
                    .saturating_mul(std::mem::size_of::<(Text, Value)>())
                    .saturating_add(data.index_entries_for_budget().saturating_mul(128))
                    .saturating_add(DictStorage::budget_bytes() + 16),
                ..Default::default()
            };
            for (key, value) in self.iter() {
                result.bytes = result.bytes.saturating_add(key.len());
                result.resident = result.resident.saturating_add(key.len() + 16);
                result.add(value.metrics());
            }
            result
        })
    }
}
impl FromIterator<(String, Value)> for Dict {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(iter: T) -> Self {
        let mut result = Self::default();
        for (k, v) in iter {
            result.insert(k, v);
        }
        result
    }
}
impl fmt::Debug for Dict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Dict")
            .field(&self.storage().entries)
            .finish()
    }
}
impl PartialEq for Dict {
    fn eq(&self, other: &Self) -> bool {
        self.storage().entries == other.storage().entries
    }
}

#[cfg(feature = "serde")]
mod wire {
    use super::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    impl<T: Serialize> Serialize for Shared<T> {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            self.as_ref().serialize(s)
        }
    }
    impl<'de, T: Deserialize<'de>> Deserialize<'de> for Shared<T> {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            T::deserialize(d).map(Self::new)
        }
    }
    impl Serialize for Text {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            self.as_str().serialize(s)
        }
    }
    impl<'de> Deserialize<'de> for Text {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            String::deserialize(d).map(Into::into)
        }
    }
    impl Serialize for Array {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            self.0.values.serialize(s)
        }
    }
    impl<'de> Deserialize<'de> for Array {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            Vec::<Value>::deserialize(d).map(Into::into)
        }
    }
    impl Serialize for Dict {
        fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
            self.storage().entries.serialize(s)
        }
    }
    impl<'de> Deserialize<'de> for Dict {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            let entries: Vec<(Text, Value)> = Vec::deserialize(d)?;
            let budget_capacity = entries.capacity();
            Ok(Self::from_entries(entries, budget_capacity))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dictionaries_detach_and_keep_the_conservative_budget() {
        assert_eq!(std::mem::size_of::<Dict>(), std::mem::size_of::<usize>());
        let empty = Dict::default();
        for mut dict in [
            Dict::default(),
            DictBuilder::default().finish(),
            std::iter::empty::<(String, Value)>().collect(),
            empty.clone(),
        ] {
            assert!(dict.shares_storage(&empty));
            assert_eq!(dict.metrics().resident, DictStorage::budget_bytes() + 16);
            assert_eq!(dict.metrics().bytes, 0);
            assert!(!dict.metrics().data && !dict.metrics().path);
            let (id, weak) = dict.weak_identity();
            assert_eq!(id, 0);
            assert!(weak.upgrade().is_none());
            dict.insert("x", Value::Int(1));
            assert!(!dict.shares_storage(&empty));
            assert_eq!(dict.storage().entries.capacity(), 1);
            assert_eq!(dict.storage().budget_capacity, 4);
            assert!(empty.get("x").is_none());
            assert_eq!(dict.take().get("x"), Some(&Value::Int(1)));
            assert!(dict.shares_storage(&empty));
        }
    }

    #[test]
    fn nonempty_dictionary_weak_identity_expires_and_mutation_still_detaches() {
        let mut dict = Dict::default();
        dict.insert("x", Value::Int(1));
        let (id, weak) = dict.weak_identity();
        assert_ne!(id, 0);
        let retained = dict.clone();
        dict.insert("x", Value::Int(2));
        assert_ne!(dict.weak_identity().0, id);
        assert_eq!(retained.get("x"), Some(&Value::Int(1)));
        drop(retained);
        assert!(weak.upgrade().is_none());
        let (_, weak) = dict.weak_identity();
        dict.insert("x", Value::Int(3));
        assert!(weak.upgrade().is_none());
        assert_eq!(dict.get("x"), Some(&Value::Int(3)));
        let (_, weak) = dict.weak_identity();
        drop(dict);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn empty_dictionary_observation_cannot_hide_later_data_retention() {
        use crate::data::{DataFormat, DataLimits, DataRetention};
        let mut retention = DataRetention::default();
        let empty = Dict::default();
        retention.observe(&Value::Dict(empty.clone()), 0).unwrap();
        assert!(retention.is_empty());
        let data = crate::data::parse(DataFormat::Json, b"123", DataLimits::default()).unwrap();
        let mut changed = empty.clone();
        changed.insert("data", Value::Data(data));
        assert!(retention.observe(&Value::Dict(changed.clone()), 0).is_err());
        retention
            .observe(&Value::Dict(changed.clone()), usize::MAX)
            .unwrap();
        let remaining = retention.remaining(usize::MAX);
        assert!(remaining < usize::MAX);
        retention
            .observe(&Value::Dict(changed), usize::MAX)
            .unwrap();
        assert_eq!(retention.remaining(usize::MAX), remaining);
        assert!(!empty.metrics().data);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn empty_dictionary_wire_and_deserializer_capacity_keep_their_budgets() {
        use serde::Deserialize;
        let empty = Dict::default();
        let decoded: Dict = serde_json::from_str("[]").unwrap();
        assert!(empty.shares_storage(&decoded));
        assert_eq!(serde_json::to_string(&empty).unwrap(), "[]");

        // SeqAccess can reserve slots even if it ultimately yields no entries.
        // Such a value must retain its previous conservative capacity charge.
        struct ReservedEmpty;
        impl<'de> serde::de::SeqAccess<'de> for ReservedEmpty {
            type Error = serde::de::value::Error;
            fn next_element_seed<T: serde::de::DeserializeSeed<'de>>(
                &mut self,
                _: T,
            ) -> Result<Option<T::Value>, Self::Error> {
                Ok(None)
            }
            fn size_hint(&self) -> Option<usize> {
                Some(4)
            }
        }
        let decoded =
            Dict::deserialize(serde::de::value::SeqAccessDeserializer::new(ReservedEmpty)).unwrap();
        assert!(decoded.is_empty());
        assert!(!decoded.shares_storage(&empty));
        assert_eq!(decoded.storage().budget_capacity, 4);
        assert_eq!(
            decoded.metrics().resident,
            reference_dict_resident(&decoded.storage().entries)
        );
        assert_eq!(serde_json::to_string(&decoded).unwrap(), "[]");
    }

    #[test]
    fn owned_dictionary_construction_shares_keys_and_preserves_budget_and_updates() {
        for count in [0, 1, 4, 8, 9, 16, 32] {
            let mut builder = DictBuilder::default();
            let mut reference = Dict::default();
            let keys: Vec<Text> = (0..count).map(|i| format!("field-{i}").into()).collect();
            for (i, key) in keys.iter().enumerate() {
                builder.insert(key.clone(), Value::Int(i as i64));
                reference.insert(key, Value::Int(i as i64));
            }
            if let Some(key) = keys.first() {
                builder.insert(key.clone(), Value::Str("updated".into()));
                reference.insert(key, Value::Str("updated".into()));
            }
            let dict = builder.finish();
            assert_eq!(dict, reference);
            assert_eq!(dict.metrics().bytes, reference.metrics().bytes);
            assert_eq!(dict.metrics().resident, reference.metrics().resident);
            for ((stored, _), key) in dict.storage().entries.iter().zip(&keys) {
                assert!(stored.shares_storage(key));
            }
            for key in &keys {
                assert_eq!(dict.get(key), reference.get(key));
            }
            let mut copy = dict.clone();
            copy.insert("new", Value::Bool(true));
            assert!(dict.get("new").is_none());
        }
    }

    fn reference_dict_resident(entries: &Vec<(Text, Value)>) -> usize {
        let keys: BTreeSet<_> = entries.iter().map(|(key, _)| key.as_str()).collect();
        entries.capacity() * std::mem::size_of::<(Text, Value)>()
            + keys.len() * 128
            + DictStorage::budget_bytes()
            + 16
            + entries
                .iter()
                .map(|(key, value)| key.len() + 16 + value.metrics().resident)
                .sum::<usize>()
    }

    #[test]
    fn borrowed_dict_keys_keep_storage_order_and_cached_metrics_on_update() {
        struct BorrowedKey<'a>(&'a str);
        impl AsRef<str> for BorrowedKey<'_> {
            fn as_ref(&self) -> &str {
                self.0
            }
        }

        for len in [1, 8, 9, 16] {
            let mut dict: Dict = (0..len)
                .map(|i| (format!("key-{i}"), Value::Int(i)))
                .collect();
            let retained = dict.clone();
            let key = dict.storage().entries[0].0.clone();
            let before = dict.metrics();
            dict.insert(BorrowedKey("key-0"), Value::Str("longer value".into()));
            assert!(dict.storage().entries[0].0.shares_storage(&key));
            assert_eq!(dict.iter().next().unwrap().0, "key-0");
            assert_eq!(retained.get("key-0"), Some(&Value::Int(0)));
            assert_eq!(retained.metrics().resident, before.resident);
            assert_eq!(
                dict.metrics().resident,
                reference_dict_resident(&dict.storage().entries)
            );
            let name = String::from("new-key");
            dict.insert(&name, Value::Bool(true));
            dict.insert(Text::from("new-key"), Value::Bool(false));
            assert_eq!(dict.iter().last(), Some(("new-key", &Value::Bool(false))));
            assert_eq!(dict.storage().entries.len(), len as usize + 1);
        }
    }

    #[test]
    fn compact_dict_capacity_keeps_previous_growth_and_cow_budget() {
        assert!(std::mem::size_of::<DictStorage>() <= DictStorage::budget_bytes());
        let mut dict = Dict::default();
        dict.insert("key-0", Value::Int(0));
        assert_eq!(dict.storage().entries.capacity(), 1);
        assert_eq!(dict.storage().budget_capacity, 4);
        for len in 0..66 {
            let mut dict = Dict::default();
            let mut reference = Vec::new();
            for i in 0..len {
                let key = format!("key-{i}");
                dict.insert(key.clone(), Value::Int(i));
                reference.push((Text::from(key), Value::Int(i)));
            }
            assert_eq!(dict.metrics().resident, reference_dict_resident(&reference));
            let untouched = dict.clone();
            // Both old and new storage clones keep cached metrics until mutation.
            let mut reference = reference.clone();
            if len > 0 {
                dict.insert("key-0", Value::Int(-1));
                reference[0].1 = Value::Int(-1);
                assert_eq!(dict.metrics().resident, reference_dict_resident(&reference));
                assert_eq!(untouched.get("key-0"), Some(&Value::Int(0)));
            }
            for i in len..len + 10 {
                let key = format!("key-{i}");
                dict.insert(key.clone(), Value::Int(i));
                reference.push((Text::from(key), Value::Int(i)));
                assert_eq!(dict.storage().budget_capacity, reference.capacity());
                assert_eq!(dict.metrics().resident, reference_dict_resident(&reference));
            }
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn compact_dict_deserialization_retains_vec_budget_and_duplicate_order() {
        use serde::Deserialize;
        for len in [0, 1, 2, 8, 9, 16, 33] {
            let wire = serde_json::Value::Array(
                (0..len)
                    .map(|i| serde_json::json!([format!("key-{}", i % 2), {"Int": i}]))
                    .collect(),
            );
            let reference = Vec::<(Text, Value)>::deserialize(wire.clone()).unwrap();
            let dict = Dict::deserialize(wire.clone()).unwrap();
            assert_eq!(dict.storage().budget_capacity, reference.capacity());
            assert_eq!(dict.metrics().resident, reference_dict_resident(&reference));
            assert_eq!(serde_json::to_value(&dict).unwrap(), wire);
            let mut changed = dict.clone();
            let mut reference = reference.clone();
            changed.insert("new", Value::Bool(true));
            reference.push(("new".into(), Value::Bool(true)));
            assert_eq!(
                changed.metrics().resident,
                reference_dict_resident(&reference)
            );
        }
    }

    #[test]
    fn small_dict_lookup_order_and_cow_survive_the_index_threshold() {
        let mut dict = Dict::default();
        let mut expected = Vec::new();
        for i in 0..40 {
            let key = format!("clé-{i:02}");
            dict.insert(key.clone(), Value::Int(i));
            expected.push((key, Value::Int(i)));
            assert_eq!(
                dict.iter()
                    .map(|(k, v)| (k.to_owned(), v.clone()))
                    .collect::<Vec<_>>(),
                expected
            );
            for (key, value) in &expected {
                assert_eq!(dict.get(key), Some(value));
            }
            assert_eq!(dict.get("missing"), None);
            assert_eq!(
                dict.storage().index.is_none(),
                expected.len() <= DICT_LINEAR_LIMIT
            );
            let mut changed = dict.clone();
            assert!(dict.shares_storage(&changed));
            changed.insert("clé-00", Value::Int(-1));
            assert_eq!(dict.get("clé-00"), Some(&Value::Int(0)));
            assert_eq!(changed.get("clé-00"), Some(&Value::Int(-1)));
        }
    }

    #[test]
    fn absent_small_index_keeps_the_previous_resident_budget_proxy() {
        for len in 0..32 {
            let dict: Dict = (0..len)
                .map(|i| (format!("key-{i}"), Value::Int(i)))
                .collect();
            let mut reference = Vec::<(Text, Value)>::new();
            for i in 0..len {
                reference.push((format!("key-{i}").into(), Value::Int(i)));
            }
            assert_eq!(dict.storage().budget_capacity, reference.capacity());
            let old_resident = reference.capacity() * std::mem::size_of::<(Text, Value)>()
                + len as usize * 128
                + DictStorage::budget_bytes()
                + 16
                + dict
                    .iter()
                    .map(|(key, value)| key.len() + 16 + value.metrics().resident)
                    .sum::<usize>();
            assert_eq!(dict.metrics().resident, old_resident);
            assert_eq!(dict.storage().index_entries_for_budget(), len as usize);
        }
    }

    #[cfg(feature = "serde")]
    #[test]
    fn small_and_indexed_duplicate_wire_entries_keep_first_lookup_and_full_order() {
        for len in [2, 8, 9, 16] {
            let mut wire = serde_json::Value::Array(
                (0..len)
                    .map(|i| serde_json::json!([format!("key-{}", i % 2), {"Int": i}]))
                    .collect(),
            );
            let mut dict: Dict = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(dict.get("key-0"), Some(&Value::Int(0)));
            assert_eq!(serde_json::to_value(&dict).unwrap(), wire);
            assert_eq!(dict.storage().index_entries_for_budget(), 2);
            dict.insert("key-0", Value::Int(42));
            wire[0][1] = serde_json::json!({"Int": 42});
            assert_eq!(serde_json::to_value(&dict).unwrap(), wire);
            dict.insert("new", Value::Bool(true));
            wire.as_array_mut()
                .unwrap()
                .push(serde_json::json!(["new", {"Bool": true}]));
            assert_eq!(serde_json::to_value(&dict).unwrap(), wire);
            assert_eq!(dict.get("new"), Some(&Value::Bool(true)));
            assert_eq!(dict.storage().index_entries_for_budget(), 3);
        }
    }

    #[test]
    fn shared_values_clone_only_on_shared_mutation() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[derive(Debug)]
        struct Observed {
            value: usize,
            clones: Arc<AtomicUsize>,
        }
        impl Clone for Observed {
            fn clone(&self) -> Self {
                self.clones.fetch_add(1, Ordering::Relaxed);
                Self {
                    value: self.value,
                    clones: self.clones.clone(),
                }
            }
        }
        let clones = Arc::new(AtomicUsize::new(0));
        let mut original = Shared::new(Observed {
            value: 1,
            clones: clones.clone(),
        });
        original.value = 2;
        assert_eq!(clones.load(Ordering::Relaxed), 0);
        let mut changed = original.clone();
        assert!(changed.shares_storage(&original));
        assert_eq!(clones.load(Ordering::Relaxed), 0);
        changed.value = 3;
        assert!(!changed.shares_storage(&original));
        changed.value = 4;
        assert_eq!(clones.load(Ordering::Relaxed), 1);
        assert_eq!(original.value, 2);
        assert_eq!(changed.into_inner().value, 4);
        assert_eq!(clones.load(Ordering::Relaxed), 1);
        let retained = original.clone();
        assert_eq!(original.into_inner().value, 2);
        assert_eq!(retained.value, 2);
        assert_eq!(clones.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn shared_values_keep_value_equality_debugging_and_box_conversion() {
        let original: Shared<Vec<i32>> = Shared::from(Box::new(vec![1, 2]));
        let equal = Shared::new(vec![1, 2]);
        assert_eq!(original, equal);
        assert!(!original.shares_storage(&equal));
        assert_eq!(format!("{original:?}"), "[1, 2]");
        let nan = Shared::new(Value::Float(f64::NAN));
        assert_ne!(nan, nan.clone());
        // Sharing does not require the contained value to implement Clone.
        let non_clone = Shared::new(Mutex::new(1));
        assert!(non_clone.shares_storage(&non_clone.clone()));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn shared_value_wire_format_does_not_expose_ownership() {
        let value = Shared::new(vec![1, 2, 3]);
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(json, "[1,2,3]");
        let decoded: Shared<Vec<i32>> = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, value);
        assert!(!decoded.shares_storage(&value));

        use crate::{
            builtins::Accepts,
            definitions::{FunctionDef, FunctionId},
        };
        let boxed = Box::new(FunctionDef::new(
            FunctionId::new("demo", "view"),
            Accepts::Inline,
            Type::InlineContent,
        ));
        let shared: Shared<FunctionDef> = boxed.clone().into();
        let json = serde_json::to_string(&shared).unwrap();
        assert_eq!(json, serde_json::to_string(&boxed).unwrap());
        assert_eq!(
            serde_json::from_str::<Shared<FunctionDef>>(&json).unwrap(),
            shared
        );
    }

    #[test]
    fn clones_share_storage_and_dict_mutation_detaches_without_reordering() {
        let text: Text = "x".repeat(1024 * 1024).into();
        assert!(text.shares_storage(&text.clone()));
        let array: Array = vec![Value::Str(text)].into();
        assert!(array.shares_storage(&array.clone()));
        let mut dict: Dict = [
            ("b".into(), Value::Array(array)),
            ("a".into(), Value::Int(1)),
        ]
        .into_iter()
        .collect();
        let old = dict.clone();
        assert!(dict.shares_storage(&old));
        let size = Value::Dict(dict.clone()).logical_size();
        dict.insert("b", Value::Int(2));
        assert!(!dict.shares_storage(&old));
        assert_eq!(dict.iter().map(|(k, _)| k).collect::<Vec<_>>(), ["b", "a"]);
        assert!(matches!(old.get("b"), Some(Value::Array(_))));
        assert!(Value::Dict(old).logical_size() > size / 2);
        assert!(Value::Dict(dict).logical_size() < size / 2);
    }
    #[test]
    fn metrics_and_array_validation_are_shared_but_not_pointer_equality() {
        let array: Array = (0..100_000).map(Value::Int).collect();
        let copy = array.clone();
        assert!(array.accepts(&Type::Int));
        assert!(copy.0.validated.lock().unwrap().types.contains(&Type::Int));
        assert!(Type::Array(Some(Box::new(Type::Int))).accepts(&Value::Array(copy)));
        assert_eq!(Value::Array(array.clone()).logical_size(), 1_600_000);
        let nan: Array = vec![Value::Float(f64::NAN)].into();
        assert_ne!(nan, nan.clone());
        let a: Dict = [("a".into(), Value::Int(1)), ("b".into(), Value::Int(2))]
            .into_iter()
            .collect();
        let b: Dict = [("b".into(), Value::Int(2)), ("a".into(), Value::Int(1))]
            .into_iter()
            .collect();
        assert_ne!(a, b);
        let repeated: Array = vec![Value::Array(array.clone()), Value::Array(array)].into();
        assert_eq!(Value::Array(repeated).logical_size(), 3_200_016);
    }
}
