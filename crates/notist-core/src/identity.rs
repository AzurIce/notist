//! Shared numeric mechanics only; domains own their counters, names and lifetimes.
use std::{
    fmt,
    marker::PhantomData,
    num::NonZeroU32,
    sync::atomic::{AtomicU32, Ordering},
};

/// A typed, nonzero 32-bit number. Kept private behind domain-specific wrappers.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct Id<Tag>(NonZeroU32, PhantomData<fn() -> Tag>);

impl<Tag> Id<Tag> {
    /// Only for domain-reserved constants and the allocator, never a public cast.
    pub(crate) const fn new(raw: u32) -> Self {
        Self(
            NonZeroU32::new(raw).expect("identity must be nonzero"),
            PhantomData,
        )
    }
}

impl<Tag> fmt::Debug for Id<Tag> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Explicitly instantiated per domain; generic statics would share a counter.
pub(crate) struct IdAllocator<Tag> {
    next: AtomicU32,
    marker: PhantomData<fn() -> Tag>,
}

impl<Tag> IdAllocator<Tag> {
    pub(crate) const fn new(first: u32) -> Self {
        assert!(first != 0, "identity allocation must start above zero");
        Self {
            next: AtomicU32::new(first),
            marker: PhantomData,
        }
    }

    /// Retired numbers are never reused. MAX is the exhausted sentinel, so an
    /// exhausted counter stays exhausted instead of wrapping into another ID.
    pub(crate) fn allocate(&self) -> Option<Id<Tag>> {
        self.next
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .ok()
            .map(Id::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum First {}
    #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    enum Second {}

    #[test]
    fn typed_numbers_preserve_width_niche_order_and_debug() {
        assert_eq!(size_of::<Id<First>>(), 4);
        assert_eq!(size_of::<Option<Id<First>>>(), 4);
        assert_eq!(format!("{:?}", Id::<First>::new(7)), "7");
        assert!(Id::<First>::new(7) < Id::new(8));
    }

    #[test]
    fn counters_are_explicitly_independent_and_respect_reserved_numbers() {
        let first = IdAllocator::<First>::new(2);
        let second = IdAllocator::<Second>::new(1);
        assert!(first.allocate() == Some(Id::new(2)));
        assert!(second.allocate() == Some(Id::new(1)));
        assert!(first.allocate() == Some(Id::new(3)));
    }

    #[test]
    fn exhausted_counters_never_wrap() {
        let counter = IdAllocator::<First>::new(u32::MAX - 1);
        assert!(counter.allocate() == Some(Id::new(u32::MAX - 1)));
        assert!(counter.allocate().is_none());
        assert!(counter.allocate().is_none());
    }

    #[test]
    fn concurrent_allocation_is_unique() {
        let counter = std::sync::Arc::new(IdAllocator::<First>::new(1));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let counter = counter.clone();
                std::thread::spawn(move || {
                    (0..1000)
                        .map(|_| counter.allocate().unwrap())
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let ids: std::collections::BTreeSet<_> = workers
            .into_iter()
            .flat_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(ids.len(), 8000);
        assert!(counter.allocate() == Some(Id::new(8001)));
    }

    #[test]
    #[should_panic(expected = "identity allocation must start above zero")]
    fn zero_is_not_a_valid_allocator_start() {
        let _ = IdAllocator::<First>::new(0);
    }
}
