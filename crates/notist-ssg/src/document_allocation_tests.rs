use super::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct Counting;
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
fn record() {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
    }
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn outline_allocations(root: &mut Item) -> usize {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.set(false);
        }
    }
    let mut ids = BTreeSet::new();
    let mut out = Vec::new();
    ALLOCATIONS.set(0);
    assert!(!ACTIVE.replace(true));
    let guard = Guard;
    headings(root, None, &mut ids, &mut out);
    drop(guard);
    assert!(ids.is_empty() && out.is_empty());
    ALLOCATIONS.get()
}

#[test]
fn outline_does_not_reallocate_empty_storage_per_leaf() {
    for count in [0, 1, 1024] {
        let mut root = Item::new(Ctor::Doc, Default::default()).with_children(
            (0..count)
                .map(|_| Item::new(Ctor::Node, Default::default()))
                .collect(),
        );
        // Unique nonempty storage needs no COW; empty leaf storage is shared.
        assert_eq!(outline_allocations(&mut root), 0);
        let retained = root.clone();
        // A shared parent needs an Arc and child buffer, not one Arc per leaf.
        assert_eq!(
            outline_allocations(&mut root),
            if count == 0 { 0 } else { 2 }
        );
        assert_eq!(root, retained);
    }
}
