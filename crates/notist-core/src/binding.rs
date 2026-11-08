//! Lexical identities have snapshot ownership, not a global name interner.
use crate::{
    definition_identity::FunctionName,
    definitions::{DefId, FunctionId},
    identity::{Id, IdAllocator},
    source::{Source, SourceId, Sources},
};
use rowan::TextRange;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum BindingTag {}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum TypeParamTag {}
/// A resolved lexical declaration, never interchangeable with a function ID.
/// ```compile_fail
/// fn confuse(id: notist_core::binding::BindingId) -> notist_core::definitions::DefId { id }
/// ```
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BindingId(Id<BindingTag>);
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeParamId(Id<TypeParamTag>);

// Independent programs and cached specializations may be merged. Unique numbers
// avoid rebasing IR on merge; only these counters, never names, are global.
static NEXT_BINDING: IdAllocator<BindingTag> = IdAllocator::new(1);
static NEXT_TYPE: IdAllocator<TypeParamTag> = IdAllocator::new(1);

pub(crate) type BindingRegistry =
    BTreeMap<(Option<DefId>, u32, u32, u32, u32, String), Weak<BindingInfo>>;
#[derive(Debug)]
pub struct BindingInfo {
    id: BindingId,
    pub name: String,
    pub source: SourceId,
    pub span: TextRange,
    pub scope: TextRange,
    pub owner: Option<FunctionId>,
    registry: Arc<Mutex<BindingRegistry>>,
}
impl PartialEq for BindingInfo {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for BindingInfo {}
impl Drop for BindingInfo {
    fn drop(&mut self) {
        let key = (
            self.owner.as_ref().map(FunctionId::def_id),
            u32::from(self.span.start()),
            u32::from(self.span.end()),
            u32::from(self.scope.start()),
            u32::from(self.scope.end()),
            self.name.clone(),
        );
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if registry
            .get(&key)
            .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), self))
        {
            registry.remove(&key);
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bindings(Arc<BTreeMap<BindingId, Arc<BindingInfo>>>);
impl Bindings {
    pub fn declare(
        &mut self,
        name: &str,
        source: &Source,
        span: TextRange,
    ) -> Result<BindingId, &'static str> {
        self.declare_scoped(name, source, span, span)
    }
    pub fn declare_scoped(
        &mut self,
        name: &str,
        source: &Source,
        span: TextRange,
        scope: TextRange,
    ) -> Result<BindingId, &'static str> {
        self.declare_with_owner(name, source, span, scope, None)
    }
    pub fn declare_with_owner(
        &mut self,
        name: &str,
        source: &Source,
        span: TextRange,
        scope: TextRange,
        owner: Option<&FunctionId>,
    ) -> Result<BindingId, &'static str> {
        if !scope.contains_range(span) {
            return Err("binding declaration lies outside its scope");
        }
        let registry = source.binding_registry();
        let key = (
            owner.map(FunctionId::def_id),
            u32::from(span.start()),
            u32::from(span.end()),
            u32::from(scope.start()),
            u32::from(scope.end()),
            name.to_owned(),
        );
        let owner = {
            let mut entries = registry
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(owner) = entries.get(&key).and_then(Weak::upgrade) {
                owner
            } else {
                let owner = Arc::new(BindingInfo {
                    id: BindingId::allocate()?,
                    name: name.into(),
                    source: source.id(),
                    span,
                    scope,
                    owner: owner.cloned(),
                    registry: registry.clone(),
                });
                entries.insert(key, Arc::downgrade(&owner));
                owner
            }
        };
        let id = owner.id;
        if !self.0.contains_key(&id) {
            Arc::make_mut(&mut self.0).insert(id, owner);
        }
        Ok(id)
    }
    pub fn get(&self, id: BindingId) -> Result<&BindingInfo, &'static str> {
        self.0
            .get(&id)
            .map(Arc::as_ref)
            .ok_or("binding identity is not owned by this snapshot")
    }
    pub fn find(
        &self,
        name: &str,
        source: SourceId,
        span: TextRange,
        scope: TextRange,
        owner: Option<&FunctionName>,
    ) -> Option<BindingId> {
        self.0
            .iter()
            .find(|(_, info)| {
                info.name == name
                    && info.source == source
                    && info.span == span
                    && info.scope == scope
                    && info.owner.as_deref() == owner
            })
            .map(|(id, _)| *id)
    }
    pub fn merge(&mut self, other: &Self) {
        if Arc::ptr_eq(&self.0, &other.0) || other.0.is_empty() {
            return;
        }
        if self.0.is_empty() {
            *self = other.clone();
            return;
        }
        for (id, info) in other.0.iter() {
            if !self.0.contains_key(id) {
                Arc::make_mut(&mut self.0).insert(*id, info.clone());
            }
        }
    }
    /// Cache projections must not keep caller-local declaration metadata after
    /// dropping the caller's source snapshot. Referenced bindings are validated
    /// separately before the projected program can enter a cache.
    pub fn retain_sources(&mut self, sources: &Sources) {
        let owned = |info: &BindingInfo| {
            info.source == SourceId::DETACHED || sources.source(info.source).is_ok()
        };
        if self.0.values().all(|info| owned(info)) {
            return;
        }
        Arc::make_mut(&mut self.0).retain(|_, info| owned(info));
    }
}
impl BindingId {
    pub(crate) fn allocate() -> Result<Self, &'static str> {
        NEXT_BINDING
            .allocate()
            .map(Self)
            .ok_or("32-bit binding identity space exhausted")
    }
}
impl TypeParamId {
    pub(crate) fn allocate() -> Result<Self, &'static str> {
        NEXT_TYPE
            .allocate()
            .map(Self)
            .ok_or("32-bit type parameter identity space exhausted")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lexical_ids_are_32_bit_and_domain_separated() {
        assert_eq!(size_of::<BindingId>(), 4);
        assert_eq!(size_of::<Option<BindingId>>(), 4);
        assert_eq!(size_of::<TypeParamId>(), 4);
        assert_eq!(size_of::<Option<TypeParamId>>(), 4);
    }
    #[test]
    fn source_scoped_weak_entries_share_live_declarations_and_reclaim_names() {
        let source = Source::metadata("binding-retirement.notc").unwrap();
        let same = Source::metadata("binding-retirement.notc").unwrap();
        let other = Source::metadata("binding-other.notc").unwrap();
        let mut first = Bindings::default();
        let mut second = Bindings::default();
        let span = TextRange::new(1.into(), 2.into());
        let id = first.declare("x", &source, span).unwrap();
        assert_eq!(second.declare("x", &same, span).unwrap(), id);
        assert_ne!(second.declare("x", &other, span).unwrap(), id);
        assert_ne!(
            second
                .declare("x", &source, TextRange::new(3.into(), 4.into()))
                .unwrap(),
            id
        );
        drop(first);
        assert!(second.get(id).is_ok());
        drop(second);
        assert!(source.binding_registry().lock().unwrap().is_empty());
        let mut next = Bindings::default();
        assert_ne!(next.declare("x", &source, span).unwrap(), id);
    }
    #[test]
    fn snapshots_detach_and_do_not_resolve_by_spelling() {
        let source = Source::metadata("binding-cow.notc").unwrap();
        let mut first = Bindings::default();
        let x = first
            .declare("x", &source, TextRange::empty(1.into()))
            .unwrap();
        let mut second = first.clone();
        let shadow = second
            .declare("x", &source, TextRange::empty(2.into()))
            .unwrap();
        assert!(first.get(shadow).is_err());
        assert_eq!(
            first.find(
                "x",
                source.id(),
                TextRange::empty(1.into()),
                TextRange::empty(1.into()),
                None
            ),
            Some(x)
        );
        assert_ne!(
            second.find(
                "x",
                source.id(),
                TextRange::empty(2.into()),
                TextRange::empty(2.into()),
                None
            ),
            Some(x)
        );
    }
    #[test]
    fn lexical_declarations_do_not_have_a_16_bit_limit() {
        let source = Source::metadata("binding-width.notc").unwrap();
        let mut bindings = Bindings::default();
        let ids: std::collections::BTreeSet<_> = (0..70_000u32)
            .map(|offset| {
                bindings
                    .declare("x", &source, TextRange::empty(offset.into()))
                    .unwrap()
            })
            .collect();
        assert_eq!(ids.len(), 70_000);
        drop(bindings);
        assert!(source.binding_registry().lock().unwrap().is_empty());
    }
    #[test]
    fn source_projection_drops_only_unowned_metadata_and_detaches() {
        let a = Source::metadata("binding-owned-a.notc").unwrap();
        let b = Source::metadata("binding-owned-b.notc").unwrap();
        let mut original = Bindings::default();
        let x = original.declare("x", &a, Default::default()).unwrap();
        let y = original.declare("y", &b, Default::default()).unwrap();
        let mut projected = original.clone();
        projected.retain_sources(&Sources::from(a));
        assert!(projected.get(x).is_ok());
        assert!(projected.get(y).is_err());
        assert!(original.get(y).is_ok());
    }
}
