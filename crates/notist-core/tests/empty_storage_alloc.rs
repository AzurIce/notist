//! Count empty construction, cloning, reads and teardown on the calling thread.
use notist_core::{
    item::{Ctor, Item, Value},
    shared::{Dict, DictBuilder, Shared},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
};

struct Counting;
thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    static DEALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
fn record(counter: &'static std::thread::LocalKey<Cell<usize>>) {
    if ACTIVE.try_with(Cell::get).unwrap_or(false) {
        let _ = counter.try_with(|n| n.set(n.get() + 1));
    }
}
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(&ALLOCATIONS);
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record(&ALLOCATIONS);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record(&ALLOCATIONS);
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(&DEALLOCATIONS);
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn operations(run: impl FnOnce()) -> (usize, usize) {
    struct Guard;
    impl Drop for Guard {
        fn drop(&mut self) {
            ACTIVE.set(false);
        }
    }
    ALLOCATIONS.set(0);
    DEALLOCATIONS.set(0);
    assert!(!ACTIVE.replace(true));
    let guard = Guard;
    run();
    drop(guard);
    (ALLOCATIONS.get(), DEALLOCATIONS.get())
}

#[test]
fn empty_dict_construction_clone_read_take_and_drop_do_not_allocate() {
    assert_eq!(
        operations(|| {
            for _ in 0..1024 {
                for mut dict in [
                    Dict::default(),
                    DictBuilder::default().finish(),
                    std::iter::empty::<(String, Value)>().collect(),
                ] {
                    drop(black_box(dict.clone()));
                    assert!(dict.is_empty());
                    assert!(dict.get("missing").is_none());
                    assert_eq!(dict.iter().count(), 0);
                    black_box(Value::Dict(dict.clone()).resident_size());
                    drop(black_box(dict.take()));
                    drop(black_box(dict));
                }
            }
        }),
        (0, 0)
    );
}

#[test]
fn warmed_empty_node_construction_clone_traversal_and_drop_do_not_allocate() {
    // Initialize the builtin registry and this thread's empty child Arc.
    // Neither initialization belongs to the repeated per-node operation.
    drop(Item::new(Ctor::Node, Default::default()).with_children(Vec::new()));
    assert_eq!(
        operations(|| {
            for _ in 0..1024 {
                let node = Item::new(Ctor::Node, Default::default()).with_children(Vec::new());
                let copy = black_box(&node).clone();
                black_box(node.children());
                assert_eq!(copy.descendants().count(), 1);
                drop(black_box(copy));
                drop(black_box(node));
                drop(black_box(Shared::from_nodes(Vec::new()).into_inner()));
            }
        }),
        (0, 0)
    );
}

#[test]
fn empty_mutation_allocates_only_on_detach_and_leaves_originals_empty() {
    let empty = Dict::default();
    let mut changed = empty.clone();
    let counts = operations(|| changed.insert("x", Value::Int(1)));
    // Storage, the key and a compact one-entry buffer; no empty Arc to free.
    assert_eq!(counts, (3, 0));
    assert_eq!(operations(|| changed.insert("x", Value::Int(2))), (0, 0));
    assert!(empty.is_empty());

    let empty = Shared::empty_nodes();
    let mut changed = empty.clone();
    assert_eq!(
        operations(|| {
            black_box(&mut *changed);
        }),
        (1, 0)
    );
    assert!(empty.is_empty());
    assert_eq!(
        operations(|| {
            black_box(&mut *changed);
        }),
        (0, 0)
    );
}
