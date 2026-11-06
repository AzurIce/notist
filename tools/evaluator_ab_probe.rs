//! Disposable evaluator observer. Fixed storage: no allocation for bookkeeping.
use std::{alloc::{GlobalAlloc, Layout, System}, cell::RefCell, path::Path, time::Instant};

const N: usize = 32;
const NAMES: [&str; N] = ["run", "expr", "bind", "present", "present_call", "execute", "instance",
    "capture_call", "capture_expr", "shell", "expand", "count_forest", "validate", "data_native",
    "read_data", "data_parse", "data_decode", "scope_clone", "arguments", "dispatch", "adapt",
    "input_lookup", "final_nodes", "bind_nodes", "present_nodes", "empty_present", "native_calls",
    "code_calls", "cow_unique", "cow_shared", "key_bytes", "instance_fast_hit"];
#[derive(Clone, Copy)]
struct Metric { calls: u64, ns: u128, own: u128, allocs: u64, bytes: u64, frees: u64, freed: u64, reallocs: u64 }
impl Metric { const ZERO: Self = Self { calls: 0, ns: 0, own: 0, allocs: 0, bytes: 0, frees: 0, freed: 0, reallocs: 0 }; }
#[derive(Clone, Copy)]
struct Frame { id: usize, children: u128 }
struct State {
    mode: u8, active: bool, kind: usize, seen: [usize; 3], selected: [usize; 3],
    metrics: [Metric; N * 3], stack: [Frame; 512], depth: usize,
}
thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State {
        mode: 0, active: false, kind: 0, seen: [0; 3], selected: [0; 3],
        metrics: [Metric::ZERO; N * 3], stack: [Frame { id: 0, children: 0 }; 512], depth: 0,
    }) };
}
pub fn init() {
    let mode = match std::env::var("EVAL_AB_OBSERVER").as_deref() {
        Ok("time") => 1, Ok("alloc") => 2, Ok("count") => 3, _ => 0,
    };
    STATE.with(|s| s.borrow_mut().mode = mode);
}
pub struct Page;
pub fn page(path: &Path) -> Page {
    let kind = if path.parent().and_then(Path::file_name).is_some_and(|p| p == "page")
        || path.file_name().is_some_and(|p| p == "index.not")
            && path.parent().and_then(Path::file_name).is_some_and(|p| p == "items") { 1 }
        else if path.file_name().is_some_and(|p| p == "index.not") { 2 } else { 0 };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        assert_eq!(s.depth, 0);
        s.kind = kind;
        let selected = match s.mode { 1 => s.seen[kind] % [256, 8, 1][kind] == 0, 2 | 3 => true, _ => false };
        s.seen[kind] += 1;
        s.selected[kind] += usize::from(selected);
        s.active = selected;
    });
    Page
}
impl Drop for Page { fn drop(&mut self) { STATE.with(|s| { let mut s = s.borrow_mut(); assert_eq!(s.depth, 0); s.active = false; }); } }
pub struct Span { id: usize, started: Option<Instant>, active: bool }
pub fn span(index: usize) -> Span {
    let (active, timed, id) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if !s.active { return (false, false, 0); }
        let id = s.kind * N + index;
        let depth = s.depth;
        assert!(depth < s.stack.len());
        s.stack[depth] = Frame { id, children: 0 };
        s.depth += 1;
        s.metrics[id].calls += 1;
        (true, s.mode == 1, id)
    });
    Span { id, active, started: timed.then(Instant::now) }
}
impl Drop for Span {
    fn drop(&mut self) {
        if !self.active { return; }
        let ns = self.started.map_or(0, |t| t.elapsed().as_nanos());
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.depth -= 1;
            let f = s.stack[s.depth];
            assert_eq!(f.id, self.id);
            let m = &mut s.metrics[self.id];
            m.ns += ns;
            m.own += ns.checked_sub(f.children).expect("child exceeds parent");
            if s.depth > 0 { let parent = s.depth - 1; s.stack[parent].children += ns; }
        });
    }
}
pub fn measure<T>(index: usize, f: impl FnOnce() -> T) -> T { let _s = span(index); f() }
pub fn count(index: usize, amount: usize) { STATE.with(|s| { let mut s = s.borrow_mut(); if s.active { let id = s.kind * N + index; s.metrics[id].calls += amount as u64; } }); }
pub fn counting() -> bool { STATE.with(|s| { let s = s.borrow(); s.active && s.mode == 3 }) }
pub fn cow<T: Clone>(nodes: std::sync::Arc<Vec<T>>) -> Vec<T> {
    count(if std::sync::Arc::strong_count(&nodes) == 1 { 28 } else { 29 }, 1);
    std::sync::Arc::unwrap_or_clone(nodes)
}
fn allocation(size: usize, free: usize, realloc: bool) {
    let _ = STATE.try_with(|cell| {
        let Ok(mut s) = cell.try_borrow_mut() else { return };
        if s.mode != 2 || !s.active || s.depth == 0 { return; }
        let id = s.stack[s.depth - 1].id;
        let m = &mut s.metrics[id];
        if size > 0 { m.allocs += 1; m.bytes += size as u64; }
        if free > 0 { m.frees += 1; m.freed += free as u64; }
        m.reallocs += u64::from(realloc);
    });
}
struct Observer;
unsafe impl GlobalAlloc for Observer {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { let p = unsafe { System.alloc(l) }; if !p.is_null() { allocation(l.size(), 0, false); } p }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 { let p = unsafe { System.alloc_zeroed(l) }; if !p.is_null() { allocation(l.size(), 0, false); } p }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { allocation(0, l.size(), false); unsafe { System.dealloc(p, l) }; }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, size: usize) -> *mut u8 { let p = unsafe { System.realloc(p, l, size) }; if !p.is_null() { allocation(size, l.size(), true); } p }
}
#[global_allocator]
static ALLOCATOR: Observer = Observer;
pub struct Report;
impl Drop for Report {
    fn drop(&mut self) {
        STATE.with(|s| {
            let s = s.borrow();
            assert_eq!(s.depth, 0);
            for (kind, name) in ["item", "catalog", "home"].iter().enumerate() {
                eprintln!("[eval-pages] {name} {} {}", s.seen[kind], s.selected[kind]);
                for (index, label) in NAMES.iter().enumerate() {
                    let m = s.metrics[kind * N + index];
                    if m.calls == 0 && m.allocs == 0 { continue; }
                    eprintln!("[eval-observe] {name} {label} {} {} {} {} {} {} {} {}", m.calls, m.ns, m.own, m.allocs, m.bytes, m.frees, m.freed, m.reallocs);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn observer_counts_allocations_without_allocating_for_its_own_records() {
        STATE.with(|s| s.borrow_mut().mode = 2);
        let page = page(Path::new("content/items/1.not"));
        {
            let _root = span(0);
            let value = Box::new(std::hint::black_box(17_u64));
            std::hint::black_box(&value);
            drop(value);
        }
        drop(page);
        STATE.with(|s| {
            let s = s.borrow();
            assert_eq!(s.metrics[0].allocs, 1);
            assert_eq!(s.metrics[0].frees, 1);
            assert_eq!(s.metrics[0].bytes, 8);
            assert_eq!(s.metrics[0].freed, 8);
        });
    }
    #[test]
    fn sampled_nested_times_partition_the_root() {
        STATE.with(|s| s.borrow_mut().mode = 1);
        let page = page(Path::new("content/items/1.not"));
        {
            let _root = span(0);
            measure(1, || std::hint::black_box((0..1000).sum::<u64>()));
        }
        drop(page);
        STATE.with(|s| {
            let s = s.borrow();
            assert_eq!(s.metrics[0].own + s.metrics[1].own, s.metrics[0].ns);
            assert_eq!(s.selected, [1, 0, 0]);
        });
    }
}
