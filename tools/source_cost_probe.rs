//! Diagnostic-only allocator/scopes, injected into disposable source copies.
use std::{alloc::{GlobalAlloc, Layout, System}, cell::Cell, sync::atomic::{AtomicU64, Ordering::Relaxed}, time::Instant};
struct Allocator;
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
const N: usize = 16;
const LABELS: [&str; N] = ["outside", "register", "identity_drop", "detached", "insert", "merge", "validate_node", "validate_program", "validate_expression", "validate_walk", "by_name", "records", "texts", "revision_equal", "anonymous", "reserved"];
struct Metric { calls: AtomicU64, ns: AtomicU64, own_ns: AtomicU64, allocs: AtomicU64, bytes: AtomicU64, reallocs: AtomicU64, realloc_bytes: AtomicU64, frees: AtomicU64 }
impl Metric { const fn new() -> Self { Self { calls: AtomicU64::new(0), ns: AtomicU64::new(0), own_ns: AtomicU64::new(0), allocs: AtomicU64::new(0), bytes: AtomicU64::new(0), reallocs: AtomicU64::new(0), realloc_bytes: AtomicU64::new(0), frees: AtomicU64::new(0) } } }
static METRICS: [Metric; N] = [const { Metric::new() }; N];
static COUNTERS: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
thread_local! { static TAG: Cell<usize> = const { Cell::new(0) }; static CHILD: Cell<u64> = const { Cell::new(0) }; }
fn metric() -> &'static Metric { &METRICS[TAG.try_with(Cell::get).unwrap_or(0)] }
fn add_live(n: usize) { let live = LIVE.fetch_add(n as u64, Relaxed) + n as u64; PEAK.fetch_max(live, Relaxed); }
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { let p = unsafe { System.alloc(l) }; if !p.is_null() { add_live(l.size()); let m=metric(); m.allocs.fetch_add(1,Relaxed); m.bytes.fetch_add(l.size() as u64,Relaxed); } p }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 { let p = unsafe { System.alloc_zeroed(l) }; if !p.is_null() { add_live(l.size()); let m=metric(); m.allocs.fetch_add(1,Relaxed); m.bytes.fetch_add(l.size() as u64,Relaxed); } p }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p,l) }; LIVE.fetch_sub(l.size() as u64,Relaxed); metric().frees.fetch_add(1,Relaxed); }
    unsafe fn realloc(&self,p:*mut u8,l:Layout,n:usize)->*mut u8 { let q=unsafe{System.realloc(p,l,n)}; if !q.is_null() { if n>=l.size() { add_live(n-l.size()); } else { LIVE.fetch_sub((l.size()-n) as u64,Relaxed); } let m=metric(); m.reallocs.fetch_add(1,Relaxed); m.realloc_bytes.fetch_add(n as u64,Relaxed); } q }
}
pub fn count(index: usize, amount: usize) { COUNTERS[index].fetch_add(amount as u64, Relaxed); }
pub struct Scope { index: usize, parent: usize, child_before: u64, started: Instant }
impl Scope { pub fn new(index: usize) -> Self { METRICS[index].calls.fetch_add(1,Relaxed); Self { index, parent: TAG.with(|t| t.replace(index)), child_before: CHILD.with(|t|t.replace(0)), started: Instant::now() } } }
impl Drop for Scope { fn drop(&mut self) { let ns=self.started.elapsed().as_nanos() as u64; let child=CHILD.with(|c|c.replace(self.child_before+ns)); TAG.with(|t|t.set(self.parent)); METRICS[self.index].ns.fetch_add(ns,Relaxed); METRICS[self.index].own_ns.fetch_add(ns.saturating_sub(child),Relaxed); } }
pub fn checkpoint(name: &str) { eprintln!("[source-memory] {{\"stage\":{name:?},\"live_requested_bytes\":{},\"peak_requested_bytes\":{}}}",LIVE.load(Relaxed),PEAK.load(Relaxed)); }
pub struct Report;
impl Drop for Report { fn drop(&mut self) {
    checkpoint("after_cli_locals_drop");
    for (i,m) in METRICS.iter().enumerate() { eprintln!("[source-cost] {{\"name\":{:?},\"calls\":{},\"inclusive_ns\":{},\"self_ns\":{},\"allocs\":{},\"bytes\":{},\"reallocs\":{},\"realloc_bytes\":{},\"frees\":{}}}",LABELS[i],m.calls.load(Relaxed),m.ns.load(Relaxed),m.own_ns.load(Relaxed),m.allocs.load(Relaxed),m.bytes.load(Relaxed),m.reallocs.load(Relaxed),m.realloc_bytes.load(Relaxed),m.frees.load(Relaxed)); }
    let labels=["source_lookups","node_visits","expression_visits","origin_checks","expansion_checks","new_identities","retired_identities","new_revisions","insert_cow","merge_records","node_literal_child_edges","registry_hits","registry_attempts","name_bytes_new_identity","namespace_bytes_new_identity","registry_wait_ns"];
    for (i,label) in labels.iter().enumerate() { eprintln!("[source-count] {{\"name\":{label:?},\"count\":{}}}",COUNTERS[i].load(Relaxed)); }
} }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested_scopes_restore_the_parent_and_separate_self_time() {
        assert_eq!(TAG.with(Cell::get),0);
        let before=METRICS[13].calls.load(Relaxed);
        {
            let _parent=Scope::new(13);
            { let _child=Scope::new(14); std::hint::black_box(7); }
            assert_eq!(TAG.with(Cell::get),13);
        }
        assert_eq!(TAG.with(Cell::get),0);
        assert_eq!(METRICS[13].calls.load(Relaxed),before+1);
        assert!(METRICS[13].own_ns.load(Relaxed)<=METRICS[13].ns.load(Relaxed));
    }
    #[test]
    fn scoped_allocation_counters_include_growth_and_release() {
        let m=&METRICS[15];
        let a=m.allocs.load(Relaxed);let b=m.bytes.load(Relaxed);let r=m.reallocs.load(Relaxed);let f=m.frees.load(Relaxed);
        {
            let _scope=Scope::new(15);
            let mut data=vec![7u8;std::hint::black_box(4096)];
            data.reserve(32768);
            std::hint::black_box(&data);
        }
        assert!(m.allocs.load(Relaxed)>a);
        assert!(m.bytes.load(Relaxed)>=b+4096);
        assert!(m.reallocs.load(Relaxed)>r);
        assert!(m.frees.load(Relaxed)>f);
    }
}
