//! Count traversal allocations only, on the calling test thread. Tree building
//! and destruction deliberately stay outside the measured region.
use notist_core::item::{Ctor, Item};
use rowan::TextRange;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
};

struct CountingAllocator;

thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn record() {
    let _ = TRACKING.try_with(|tracking| {
        if tracking.get() {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
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
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn node(children: Vec<Item>) -> Item {
    Item::new(Ctor::Node, TextRange::empty(0.into())).with_children(children)
}

fn traverse(root: &Item) -> (usize, usize) {
    ALLOCATIONS.set(0);
    TRACKING.set(true);
    let mut visited = 0;
    for item in black_box(root).descendants() {
        black_box(item);
        visited += 1;
    }
    TRACKING.set(false);
    (visited, ALLOCATIONS.get())
}

#[test]
fn shallow_traversals_do_not_allocate_even_with_many_siblings() {
    let leaf = node(vec![]);
    assert_eq!(traverse(&leaf), (1, 0));

    let wide = node((0..4096).map(|_| node(vec![])).collect());
    assert_eq!(traverse(&wide), (4097, 0));

    let mut nested = node(vec![]);
    for _ in 0..7 {
        nested = node(vec![nested]);
    }
    assert_eq!(traverse(&nested), (8, 0));

    // No pending siblings means no parent cursors, regardless of depth.
    for _ in 0..121 {
        nested = node(vec![nested]);
    }
    assert_eq!(traverse(&nested), (129, 0));
}

#[test]
fn deep_traversals_fall_back_to_heap_storage() {
    let mut root = node(vec![]);
    for _ in 0..128 {
        root = node(vec![root, node(vec![])]);
    }
    let (visited, allocations) = traverse(&root);
    assert_eq!(visited, 257);
    assert!(allocations > 0);
}
