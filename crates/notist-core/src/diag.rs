use crate::source::SourceId;
use rowan::TextRange;
use std::{collections::BTreeMap, sync::Arc};

/// Source snapshots owned by an analysis/evaluation, not by the name interner.
/// Clones share the map until one result adds its own document/data sources.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sources(Arc<BTreeMap<SourceId, Arc<str>>>);

/// Lookup by identity, or by an exact source name at a host boundary.
/// This deliberately is not Borrow<str>: SourceId hashes its integer identity,
/// whereas a string hashes its bytes, so Borrow's hash contract would be false.
pub trait SourceLookup {
    fn source_id(&self) -> Option<SourceId>;
}
impl SourceLookup for SourceId {
    fn source_id(&self) -> Option<SourceId> {
        Some(*self)
    }
}
impl SourceLookup for str {
    fn source_id(&self) -> Option<SourceId> {
        SourceId::get(self)
    }
}
impl SourceLookup for String {
    fn source_id(&self) -> Option<SourceId> {
        self.as_str().source_id()
    }
}
impl Sources {
    pub fn get<Q: SourceLookup + ?Sized>(&self, key: &Q) -> Option<&std::sync::Arc<str>> {
        self.0.get(&key.source_id()?)
    }
    pub fn get_key_value<Q: SourceLookup + ?Sized>(
        &self,
        key: &Q,
    ) -> Option<(&SourceId, &std::sync::Arc<str>)> {
        self.0.get_key_value(&key.source_id()?)
    }
    pub fn contains_key<Q: SourceLookup + ?Sized>(&self, key: &Q) -> bool {
        self.get(key).is_some()
    }
    pub fn insert(
        &mut self,
        key: SourceId,
        text: std::sync::Arc<str>,
    ) -> Option<std::sync::Arc<str>> {
        std::sync::Arc::make_mut(&mut self.0).insert(key, text)
    }
    pub fn append(&mut self, other: &mut Self) {
        if !other.is_empty() {
            std::sync::Arc::make_mut(&mut self.0).append(std::sync::Arc::make_mut(&mut other.0));
        }
    }
}
impl std::ops::Deref for Sources {
    type Target = std::collections::BTreeMap<SourceId, std::sync::Arc<str>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<Q: SourceLookup + ?Sized> std::ops::Index<&Q> for Sources {
    type Output = std::sync::Arc<str>;
    fn index(&self, key: &Q) -> &Self::Output {
        self.get(key).expect("source is not part of this snapshot")
    }
}
impl<const N: usize> From<[(SourceId, std::sync::Arc<str>); N]> for Sources {
    fn from(entries: [(SourceId, std::sync::Arc<str>); N]) -> Self {
        entries.into_iter().collect()
    }
}
impl FromIterator<(SourceId, std::sync::Arc<str>)> for Sources {
    fn from_iter<I: IntoIterator<Item = (SourceId, std::sync::Arc<str>)>>(iter: I) -> Self {
        Self(std::sync::Arc::new(iter.into_iter().collect()))
    }
}
impl Extend<(SourceId, std::sync::Arc<str>)> for Sources {
    fn extend<I: IntoIterator<Item = (SourceId, std::sync::Arc<str>)>>(&mut self, iter: I) {
        std::sync::Arc::make_mut(&mut self.0).extend(iter);
    }
}
impl<'a> IntoIterator for &'a Sources {
    type Item = (&'a SourceId, &'a std::sync::Arc<str>);
    type IntoIter = std::collections::btree_map::Iter<'a, SourceId, std::sync::Arc<str>>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl IntoIterator for Sources {
    type Item = (SourceId, std::sync::Arc<str>);
    type IntoIter = std::collections::btree_map::IntoIter<SourceId, std::sync::Arc<str>>;
    fn into_iter(self) -> Self::IntoIter {
        std::sync::Arc::unwrap_or_clone(self.0).into_iter()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Sources {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (id, text) in self {
            map.serialize_entry(id, text.as_ref())?;
        }
        map.end()
    }
}
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Sources {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let entries =
            <std::collections::BTreeMap<SourceId, String> as serde::Deserialize>::deserialize(
                deserializer,
            )?;
        Ok(entries
            .into_iter()
            .map(|(id, text)| (id, text.into()))
            .collect())
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    #[test]
    fn snapshots_share_maps_until_mutated_and_names_do_not_register_on_misses() {
        let id = "snapshots/main.not".into();
        let old = Sources::from([(id, "old".into())]);
        let mut new = old.clone();
        assert!(std::sync::Arc::ptr_eq(&old.0, &new.0));
        new.insert(id, "new".into());
        assert!(!std::sync::Arc::ptr_eq(&old.0, &new.0));
        assert_eq!(old[&id].as_ref(), "old");
        assert_eq!(new["snapshots/main.not"].as_ref(), "new");
        let missing = "source-id-no-registration-on-missing-lookup.not";
        assert_eq!(SourceId::get(missing), None);
        assert!(!old.contains_key(missing));
        assert_eq!(SourceId::get(missing), None);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn source_map_wire_is_still_names_to_text_in_lexical_order() {
        let sources = Sources::from([("z.not".into(), "z".into()), ("a.not".into(), "a".into())]);
        let wire = serde_json::to_string(&sources).unwrap();
        assert_eq!(wire, "{\"a.not\":\"a\",\"z.not\":\"z\"}");
        assert_eq!(serde_json::from_str::<Sources>(&wire).unwrap(), sources);
    }
}

/// Which pipeline phase produced a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Phase {
    /// Parser: malformed structure.
    Syntax,
    /// Desugar: value-domain errors (bare names, number ranges, entries).
    Semantic,
    /// Definition validation and Resolve: signature/flavor mismatches.
    Type,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Phase::Syntax => "syntax",
            Phase::Semantic => "semantic",
            Phase::Type => "type",
        })
    }
}

/// Stable classification for host policy, independent of diagnostic wording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DiagnosticCode {
    OutsideVault,
    /// An explicit local embed has no public URL selected by its host.
    UnpublishedResource,
}

/// The shared diagnostic type of the whole pipeline: `notist-syntax`'s own
/// `parser::Diagnostic` is converted in at the boundary with `phase: Syntax`.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Diagnostic {
    #[cfg_attr(feature = "serde", serde(with = "crate::evaluation::span"))]
    pub span: TextRange,
    pub message: String,
    pub phase: Phase,
    #[cfg_attr(feature = "serde", serde(default))]
    pub code: Option<DiagnosticCode>,
    /// Source identity when expansion/checking crosses the document boundary.
    #[cfg_attr(feature = "serde", serde(default))]
    pub source: Option<SourceId>,
    /// A content boundary contains this failure and preserves independent content.
    /// Hosts may present the recovered tree regardless of the diagnostic phase.
    #[cfg_attr(feature = "serde", serde(default))]
    pub recovered: bool,
}

impl Diagnostic {
    pub fn new(phase: Phase, span: TextRange, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            phase,
            code: None,
            source: None,
            recovered: false,
        }
    }
}
