//! Shared immutable language containers. Builders use copy-on-write; wire data
//! and equality retain the previous string/vector/ordered-entry semantics.
use crate::{definitions::Type, item::Value};
use std::{
    borrow::Borrow,
    collections::{BTreeMap, BTreeSet},
    fmt,
    ops::Deref,
    sync::{Arc, Mutex, OnceLock},
};

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

#[derive(Clone, Default)]
pub struct Dict(Arc<DictStorage>);
#[derive(Clone, Default)]
pub(crate) struct DictStorage {
    entries: Vec<(Text, Value)>,
    index: BTreeMap<Text, usize>,
    metrics: OnceLock<Metrics>,
}
impl Dict {
    pub(crate) fn weak_identity(&self) -> (usize, std::sync::Weak<DictStorage>) {
        (Arc::as_ptr(&self.0) as usize, Arc::downgrade(&self.0))
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.index.get(key).map(|i| &self.0.entries[*i].1)
    }
    pub fn is_empty(&self) -> bool {
        self.0.entries.is_empty()
    }
    pub fn insert(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        let data = Arc::make_mut(&mut self.0);
        data.metrics = OnceLock::new();
        match data.index.get(key.as_str()).copied() {
            Some(i) => data.entries[i].1 = value,
            None => {
                let key: Text = key.into();
                data.index.insert(key.clone(), data.entries.len());
                data.entries.push((key, value));
            }
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.entries.iter().map(|(k, v)| (k.as_str(), v))
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
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(crate) fn metrics(&self) -> Metrics {
        *self.0.metrics.get_or_init(|| {
            let mut result = Metrics {
                resident: self
                    .0
                    .entries
                    .capacity()
                    .saturating_mul(std::mem::size_of::<(Text, Value)>())
                    .saturating_add(self.0.index.len().saturating_mul(128))
                    .saturating_add(std::mem::size_of::<DictStorage>() + 16),
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
        f.debug_tuple("Dict").field(&self.0.entries).finish()
    }
}
impl PartialEq for Dict {
    fn eq(&self, other: &Self) -> bool {
        self.0.entries == other.0.entries
    }
}

#[cfg(feature = "serde")]
mod wire {
    use super::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
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
            self.0.entries.serialize(s)
        }
    }
    impl<'de> Deserialize<'de> for Dict {
        fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            let entries: Vec<(Text, Value)> = Vec::deserialize(d)?;
            let mut index = BTreeMap::new();
            for (i, (key, _)) in entries.iter().enumerate() {
                index.entry(key.clone()).or_insert(i);
            }
            Ok(Self(Arc::new(DictStorage {
                entries,
                index,
                ..Default::default()
            })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
