//! Disposable field/call-site allocation observer. Never linked into normal A/B.
use std::{alloc::{GlobalAlloc, Layout, System}, cell::RefCell};

const FIELDS: [&str; 7] = ["ctor", "instance", "target", "evaluation", "origin", "fields", "attrs"];
const SITES: [&str; 4] = ["bind", "present_recovery", "call_recovery", "capture"];
#[derive(Clone, Copy, Default)]
struct Metric { calls: u64, allocations: u64, bytes: u64, reallocations: u64 }
struct State { site: usize, field: Option<usize>, metrics: [[Metric; 7]; 4] }
thread_local! {
    static STATE: RefCell<State> = const { RefCell::new(State {
        site: 0, field: None, metrics: [[Metric { calls: 0, allocations: 0, bytes: 0, reallocations: 0 }; 7]; 4],
    }) };
}
struct RestoreSite(usize);
impl Drop for RestoreSite {
    fn drop(&mut self) { STATE.with(|s| s.borrow_mut().site = self.0); }
}
pub fn site<T>(site: usize, f: impl FnOnce() -> T) -> T {
    let previous = STATE.with(|s| { let mut s = s.borrow_mut(); std::mem::replace(&mut s.site, site) });
    let _restore = RestoreSite(previous);
    f()
}
struct RestoreField(Option<usize>);
impl Drop for RestoreField {
    fn drop(&mut self) { STATE.with(|s| s.borrow_mut().field = self.0); }
}
pub fn field<T>(field: usize, f: impl FnOnce() -> T) -> T {
    let previous = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let site = s.site;
        s.metrics[site][field].calls += 1;
        s.field.replace(field)
    });
    let _restore = RestoreField(previous);
    f()
}
fn observe(bytes: usize, realloc: bool) {
    let _ = STATE.try_with(|s| {
        let Ok(mut s) = s.try_borrow_mut() else { return };
        let Some(field) = s.field else { return };
        let site = s.site;
        let metric = &mut s.metrics[site][field];
        metric.allocations += 1;
        metric.bytes += bytes as u64;
        metric.reallocations += u64::from(realloc);
    });
}
struct Observer;
unsafe impl GlobalAlloc for Observer {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 { let p = unsafe { System.alloc(l) }; if !p.is_null() { observe(l.size(), false); } p }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 { let p = unsafe { System.alloc_zeroed(l) }; if !p.is_null() { observe(l.size(), false); } p }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) { unsafe { System.dealloc(p, l) }; }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 { let p = unsafe { System.realloc(p, l, n) }; if !p.is_null() { observe(n, true); } p }
}
#[global_allocator]
static ALLOCATOR: Observer = Observer;
pub struct Report;
impl Drop for Report {
    fn drop(&mut self) {
        STATE.with(|s| {
            let s = s.borrow();
            assert!(s.field.is_none());
            for (i, site) in SITES.iter().enumerate() {
                for (j, field) in FIELDS.iter().enumerate() {
                    let m = s.metrics[i][j];
                    eprintln!("[boundary-field] {site} {field} {} {} {} {}", m.calls, m.allocations, m.bytes, m.reallocations);
                }
            }
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixed_records_count_only_the_selected_field_and_restore_scope() {
        site(2, || field(4, || {
            let value = Box::new(std::hint::black_box(123_u64));
            std::hint::black_box(&value);
            drop(value);
        }));
        STATE.with(|s| {
            let s = s.borrow();
            assert_eq!(s.site, 0);
            assert!(s.field.is_none());
            assert_eq!(s.metrics[2][4].calls, 1);
            assert_eq!(s.metrics[2][4].allocations, 1);
            assert_eq!(s.metrics[2][4].bytes, 8);
        });
    }
}
