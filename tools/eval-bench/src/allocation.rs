//! Rust requested-allocation diagnostics, never enabled in the timing binary.
//! The benchmark process is single-threaded. This is not RSS or allocator usable
//! size: existing fixtures/cache are the baseline, and peaks are incremental.
use serde_json::{Value, json};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
};

struct Allocator;
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
static LIVE: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Default)]
struct Stats {
    allocations: usize,
    allocation_bytes: usize,
    reallocations: usize,
    reallocation_bytes: usize,
    frees: usize,
    baseline: usize,
    peak: usize,
}

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static STATS: Cell<Stats> = const { Cell::new(Stats { allocations: 0, allocation_bytes: 0, reallocations: 0, reallocation_bytes: 0, frees: 0, baseline: 0, peak: 0 }) };
}

fn record(update: impl FnOnce(&mut Stats)) {
    if ENABLED.try_with(Cell::get).unwrap_or(false) {
        let _ = STATS.try_with(|cell| {
            let mut stats = cell.get();
            update(&mut stats);
            stats.peak = stats.peak.max(LIVE.load(Relaxed));
            cell.set(stats);
        });
    }
}

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Relaxed);
            record(|s| {
                s.allocations += 1;
                s.allocation_bytes += layout.size();
            });
        }
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            LIVE.fetch_add(layout.size(), Relaxed);
            record(|s| {
                s.allocations += 1;
                s.allocation_bytes += layout.size();
            });
        }
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(pointer, layout);
        }
        LIVE.fetch_sub(layout.size(), Relaxed);
        record(|s| s.frees += 1);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let result = unsafe { System.realloc(pointer, layout, size) };
        if !result.is_null() {
            if size >= layout.size() {
                LIVE.fetch_add(size - layout.size(), Relaxed);
            } else {
                LIVE.fetch_sub(layout.size() - size, Relaxed);
            }
            record(|s| {
                s.reallocations += 1;
                s.reallocation_bytes += size;
            });
        }
        result
    }
}

pub struct Measurement {
    stats: Stats,
    output_live: usize,
    final_live: usize,
}

impl Measurement {
    pub fn json(&self) -> Value {
        json!({
            "allocations": self.stats.allocations,
            "allocation_bytes": self.stats.allocation_bytes,
            "reallocations": self.stats.reallocations,
            "reallocation_requested_bytes": self.stats.reallocation_bytes,
            "frees": self.stats.frees,
            "peak_extra_requested_bytes": self.stats.peak.saturating_sub(self.stats.baseline),
            "output_live_extra_bytes": self.output_live as i128 - self.stats.baseline as i128,
            "net_live_after_drop_bytes": self.final_live as i128 - self.stats.baseline as i128,
        })
    }
}

pub fn measure<T>(operation: impl FnOnce() -> T) -> Measurement {
    assert!(!ENABLED.with(Cell::get), "nested allocation measurement");
    let baseline = LIVE.load(Relaxed);
    STATS.with(|cell| {
        cell.set(Stats {
            baseline,
            peak: baseline,
            ..Stats::default()
        })
    });
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ENABLED.with(|cell| cell.set(false));
        }
    }
    ENABLED.with(|cell| cell.set(true));
    let reset = Reset;
    let value = std::hint::black_box(operation());
    let output_live = LIVE.load(Relaxed);
    drop(value);
    let final_live = LIVE.load(Relaxed);
    drop(reset);
    Measurement {
        stats: STATS.with(Cell::get),
        output_live,
        final_live,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn records_successful_allocation_and_release_without_claiming_rss() {
        let measured = measure(|| vec![std::hint::black_box(7u8); 8192]).json();
        assert!(measured["allocations"].as_u64().unwrap() >= 1);
        assert!(measured["allocation_bytes"].as_u64().unwrap() >= 8192);
        assert!(measured["frees"].as_u64().unwrap() >= 1);
    }
}
