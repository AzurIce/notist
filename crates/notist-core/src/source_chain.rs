//! Copy-on-write ordered expansion locations; empty chains allocate nothing.
use crate::item::SourceLocation;
use std::{
    fmt,
    ops::{Deref, DerefMut},
    sync::Arc,
};

#[derive(Clone, Default)]
pub struct SourceChain(Option<Arc<Vec<SourceLocation>>>);

impl PartialEq for SourceChain {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}
impl Eq for SourceChain {}

impl SourceChain {
    pub fn shares_storage(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
impl From<Vec<SourceLocation>> for SourceChain {
    fn from(value: Vec<SourceLocation>) -> Self {
        if value.is_empty() {
            Self::default()
        } else {
            Self(Some(Arc::new(value)))
        }
    }
}
impl From<SourceChain> for Vec<SourceLocation> {
    fn from(value: SourceChain) -> Self {
        value.0.map(Arc::unwrap_or_clone).unwrap_or_default()
    }
}
impl Deref for SourceChain {
    type Target = Vec<SourceLocation>;
    fn deref(&self) -> &Self::Target {
        static EMPTY: Vec<SourceLocation> = Vec::new();
        self.0.as_deref().unwrap_or(&EMPTY)
    }
}
impl DerefMut for SourceChain {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(self.0.get_or_insert_with(|| Arc::new(Vec::new())))
    }
}
impl fmt::Debug for SourceChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_slice().fmt(f)
    }
}
#[cfg(feature = "serde")]
impl serde::Serialize for SourceChain {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(self.as_slice(), serializer)
    }
}
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for SourceChain {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <Vec<SourceLocation> as serde::Deserialize>::deserialize(deserializer).map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_and_nonempty_chains_detach_without_changing_order_or_spans() {
        let empty = SourceChain::default();
        let mut changed = empty.clone();
        changed.push(SourceLocation {
            source: "a.not".into(),
            start: 1,
            end: 2,
        });
        assert!(empty.is_empty());
        let original = changed.clone();
        assert!(original.shares_storage(&changed));
        changed[0].end = 9;
        changed.push(SourceLocation {
            source: "b.not".into(),
            start: 3,
            end: 4,
        });
        assert_eq!(original[0].end, 2);
        assert_eq!(original.len(), 1);
        assert_eq!(
            changed
                .iter()
                .map(|x| x.source.as_str())
                .collect::<Vec<_>>(),
            ["a.not", "b.not"]
        );
        assert!(!original.shares_storage(&changed));
        changed.clear();
        assert_eq!(empty, changed);
    }
}
