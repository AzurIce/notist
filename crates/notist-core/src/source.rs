//! Interned source names. Source identity is not a source revision or an IO grant.
//!
//! The process-local pool retains names, not source bodies. This deliberately
//! simple first implementation lets bare public trees keep their existing wire
//! representation. It is not a reclaiming source database: long-running hosts
//! with unbounded distinct source names must account for the retained metadata.
use std::{
    collections::HashMap,
    fmt,
    num::NonZeroU32,
    ops::Deref,
    sync::{LazyLock, RwLock},
};

#[derive(Default)]
struct Interner {
    ids: HashMap<&'static str, SourceId>,
    names: Vec<&'static str>,
}

static INTERNER: LazyLock<RwLock<Interner>> = LazyLock::new(|| RwLock::new(Interner::default()));

/// A 32-bit, process-local identity for an exact logical source name.
///
/// Equal names have equal IDs. Names are not normalized, resolved against disk,
/// or interpreted as authorization. Hosts must continue providing unambiguous
/// logical names (including their existing root where appropriate).
///
/// Copying, equality and hashing do not access the pool. Only registration and
/// name resolution acquire a lock. Numeric allocation order is never a stable
/// external identity; serialization and debugging expose the original name.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(NonZeroU32);

impl SourceId {
    pub fn intern(name: &str) -> Self {
        {
            let pool = INTERNER
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(id) = pool.ids.get(name) {
                return *id;
            }
        }
        let mut pool = INTERNER
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(id) = pool.ids.get(name) {
            return *id;
        }
        let index = pool
            .names
            .len()
            .checked_add(1)
            .and_then(|n| u32::try_from(n).ok())
            .and_then(NonZeroU32::new)
            .expect("source identity space exhausted");
        let id = Self(index);
        let name = Box::leak(name.to_owned().into_boxed_str());
        pool.names.push(name);
        pool.ids.insert(name, id);
        id
    }

    pub fn as_str(self) -> &'static str {
        let pool = INTERNER
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pool.names[self.0.get() as usize - 1]
    }

    /// Resolve an already registered name without growing the pool on a miss.
    pub fn get(name: &str) -> Option<Self> {
        INTERNER
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ids
            .get(name)
            .copied()
    }
}

// Keep the existing lexical order at ordered output/index boundaries. Equality
// and Hash use only the canonical integer. No ordering of raw IDs is exposed.
impl Ord for SourceId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        if self == other {
            std::cmp::Ordering::Equal
        } else {
            self.as_str().cmp(other.as_str())
        }
    }
}
impl PartialOrd for SourceId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl From<&str> for SourceId {
    fn from(name: &str) -> Self {
        Self::intern(name)
    }
}
impl Default for SourceId {
    fn default() -> Self {
        Self::intern("")
    }
}
impl From<String> for SourceId {
    fn from(name: String) -> Self {
        Self::intern(&name)
    }
}
impl From<&String> for SourceId {
    fn from(name: &String) -> Self {
        Self::intern(name)
    }
}
impl Deref for SourceId {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}
impl AsRef<str> for SourceId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}
impl fmt::Debug for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_str().fmt(f)
    }
}
impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl PartialEq<str> for SourceId {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}
impl PartialEq<&str> for SourceId {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for SourceId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for SourceId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = <String as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Self::intern(&name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_and_exact_without_path_normalization() {
        assert_eq!(std::mem::size_of::<SourceId>(), 4);
        assert_eq!(std::mem::size_of::<Option<SourceId>>(), 4);
        assert_eq!(SourceId::intern("a.not"), SourceId::intern("a.not"));
        assert_ne!(SourceId::intern("a.not"), SourceId::intern("./a.not"));
        assert_ne!(
            SourceId::intern("/one/a.not"),
            SourceId::intern("/two/a.not")
        );
        assert_eq!(format!("{:?}", SourceId::intern("a.not")), "\"a.not\"");
    }

    #[test]
    fn supports_more_than_sixteen_bits_and_concurrent_registration() {
        for index in 0..70_000 {
            let name = format!("source-id-test/{index}.not");
            let id = SourceId::intern(&name);
            assert_eq!(id.as_str(), name);
        }
        let handles: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(|| SourceId::intern("concurrent.not")))
            .collect();
        let ids: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(ids.iter().all(|id| *id == ids[0]));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn wire_is_a_source_name_not_an_integer() {
        let id = SourceId::intern("modules/view.notc");
        let encoded = serde_json::to_string(&id).unwrap();
        assert_eq!(encoded, "\"modules/view.notc\"");
        assert_eq!(serde_json::from_str::<SourceId>(&encoded).unwrap(), id);
    }
}
