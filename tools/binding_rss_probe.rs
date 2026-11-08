//! Diagnostic only. "tags" adds allocation headers; "pages" preserves layouts.
use std::{alloc::{GlobalAlloc, Layout, System}, cell::Cell, sync::atomic::{AtomicU64, Ordering::Relaxed}};
const N: usize = 16;
const LABELS: [&str; N] = ["outside", "source_identity", "source_binding_registry", "empty_binding_snapshot", "binding_declaration", "binding_snapshot_merge", "resolved_signatures", "module_scope_pool", "source_views", "runtime_scope", "checker_specialization", "registry_install", "html_registry", "reserved13", "reserved14", "reserved15"];
struct Metric { live: AtomicU64, count: AtomicU64, allocated: AtomicU64, allocs: AtomicU64 }
impl Metric { const fn new() -> Self { Self { live: AtomicU64::new(0), count: AtomicU64::new(0), allocated: AtomicU64::new(0), allocs: AtomicU64::new(0) } } }
static M: [Metric; N] = [const { Metric::new() }; N];
static LIVE: AtomicU64 = AtomicU64::new(0);
static PEAK: AtomicU64 = AtomicU64::new(0);
static USABLE: AtomicU64 = AtomicU64::new(0);
static SNAP_AT: AtomicU64 = AtomicU64::new(0);
static SNAP: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];
thread_local! { static TAG: Cell<usize> = const { Cell::new(0) }; }
const TAGGED: bool = cfg!(binding_rss_tags);
pub struct Scope(usize);
impl Scope { pub fn new(tag: usize) -> Self { Self(TAG.with(|t| t.replace(tag))) } }
impl Drop for Scope { fn drop(&mut self) { TAG.with(|t| t.set(self.0)); } }
#[repr(C)]
struct Header { tag: usize }
fn offset(l: Layout) -> usize { l.align().max(size_of::<Header>()) }
fn padded(l: Layout) -> Layout { Layout::from_size_align(l.size().checked_add(offset(l)).expect("probe layout overflow"), l.align().max(align_of::<Header>())).unwrap() }
unsafe extern "C" { fn malloc_usable_size(p: *mut std::ffi::c_void) -> usize; fn mallinfo2() -> MallInfo; fn malloc_trim(pad: usize) -> i32; }
#[repr(C)]
#[derive(Default)]
struct MallInfo { arena: usize, ordblks: usize, smblks: usize, hblks: usize, hblkhd: usize, usmblks: usize, fsmblks: usize, uordblks: usize, fordblks: usize, keepcost: usize }
fn add(tag: usize, n: usize, new: bool) {
    M[tag].live.fetch_add(n as u64, Relaxed);
    M[tag].allocated.fetch_add(n as u64, Relaxed);
    if new { M[tag].count.fetch_add(1, Relaxed); M[tag].allocs.fetch_add(1, Relaxed); }
    let live = LIVE.fetch_add(n as u64, Relaxed) + n as u64;
    let old = PEAK.fetch_max(live, Relaxed);
    if TAGGED && live > old && live > SNAP_AT.load(Relaxed) + 32768 {
        SNAP_AT.store(live, Relaxed);
        for i in 0..N { SNAP[i].store(M[i].live.load(Relaxed), Relaxed); }
    }
}
fn sub(tag: usize, n: usize, freed: bool) { M[tag].live.fetch_sub(n as u64, Relaxed); LIVE.fetch_sub(n as u64, Relaxed); if freed { M[tag].count.fetch_sub(1, Relaxed); } }
struct Allocator;
#[global_allocator] static ALLOCATOR: Allocator = Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let tag = if TAGGED { TAG.try_with(Cell::get).unwrap_or(0) } else { 0 };
        let p = unsafe { System.alloc(if TAGGED { padded(l) } else { l }) };
        if p.is_null() { return p; }
        let result = if TAGGED { unsafe { p.cast::<Header>().write(Header { tag }); p.add(offset(l)) } } else { p };
        if !TAGGED { USABLE.fetch_add(unsafe { malloc_usable_size(p.cast()) } as u64, Relaxed); }
        add(tag, l.size(), true); result
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let tag = if TAGGED { TAG.try_with(Cell::get).unwrap_or(0) } else { 0 };
        let p = unsafe { System.alloc_zeroed(if TAGGED { padded(l) } else { l }) };
        if p.is_null() { return p; }
        let result = if TAGGED { unsafe { p.cast::<Header>().write(Header { tag }); p.add(offset(l)) } } else { p };
        if !TAGGED { USABLE.fetch_add(unsafe { malloc_usable_size(p.cast()) } as u64, Relaxed); }
        add(tag, l.size(), true); result
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        let (base, tag) = if TAGGED { let base = unsafe { p.sub(offset(l)) }; (base, unsafe { (*base.cast::<Header>()).tag }) } else { (p, 0) };
        if !TAGGED { USABLE.fetch_sub(unsafe { malloc_usable_size(base.cast()) } as u64, Relaxed); }
        sub(tag, l.size(), true); unsafe { System.dealloc(base, if TAGGED { padded(l) } else { l }); }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let (base, tag) = if TAGGED { let base = unsafe { p.sub(offset(l)) }; (base, unsafe { (*base.cast::<Header>()).tag }) } else { (p, 0) };
        let old_usable = if TAGGED { 0 } else { unsafe { malloc_usable_size(base.cast()) } };
        let q = unsafe { System.realloc(base, if TAGGED { padded(l) } else { l }, if TAGGED { n.checked_add(offset(l)).expect("probe overflow") } else { n }) };
        if q.is_null() { return q; }
        if !TAGGED { let usable = unsafe { malloc_usable_size(q.cast()) }; if usable >= old_usable { USABLE.fetch_add((usable-old_usable) as u64, Relaxed); } else { USABLE.fetch_sub((old_usable-usable) as u64, Relaxed); } }
        if n >= l.size() { add(tag, n-l.size(), false); } else { sub(tag, l.size()-n, false); }
        if TAGGED { unsafe { q.add(offset(l)) } } else { q }
    }
}
fn os_snapshot() -> String {
    let info = unsafe { mallinfo2() };
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let get = |key: &str| status.lines().find_map(|line| line.strip_prefix(key)).unwrap().split_whitespace().next().unwrap().parse::<u64>().unwrap()*1024;
    format!("\"rss\":{},\"hwm\":{},\"anon\":{},\"file\":{},\"arena\":{},\"in_use\":{},\"free\":{},\"mmapped\":{},\"free_chunks\":{},\"top_free\":{}",get("VmRSS:"),get("VmHWM:"),get("RssAnon:"),get("RssFile:"),info.arena,info.uordblks,info.fordblks,info.hblkhd,info.ordblks,info.keepcost)
}
pub fn checkpoint(stage: &str) {
    let live = LIVE.load(Relaxed); let peak = PEAK.load(Relaxed); let usable = USABLE.load(Relaxed);
    let os = os_snapshot();
    eprintln!("[binding-rss] {{\"stage\":{stage:?},\"tagged\":{TAGGED},\"requested\":{live},\"peak_requested\":{peak},\"rust_usable\":{usable},{os}}}");
    if TAGGED {
        for i in 0..N { eprintln!("[binding-owner] {{\"stage\":{stage:?},\"name\":{:?},\"live\":{},\"count\":{},\"allocated\":{},\"allocs\":{},\"near_peak_live\":{},\"near_peak_at\":{}}}",LABELS[i],M[i].live.load(Relaxed),M[i].count.load(Relaxed),M[i].allocated.load(Relaxed),M[i].allocs.load(Relaxed),SNAP[i].load(Relaxed),SNAP_AT.load(Relaxed)); }
    }
    if !TAGGED && std::env::var_os("BINDING_RSS_TRIM").is_some() {
        let result = unsafe { malloc_trim(0) };
        let os = os_snapshot();
        eprintln!("[binding-trim] {{\"stage\":{stage:?},\"result\":{result},{os}}}");
    }
}
pub struct Report;
impl Drop for Report { fn drop(&mut self) { checkpoint("after_cli_locals_drop"); } }
