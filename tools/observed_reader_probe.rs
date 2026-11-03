//! Supplementary CLI-wrapper timers, only compiled in a disposable probe.
use std::{cell::RefCell, sync::OnceLock, time::Instant};
#[derive(Clone, Copy, Default)]
struct Stat { calls: u64, nanos: u128 }
thread_local! { static STATS: RefCell<[Stat; 3]> = RefCell::new([Stat::default(); 3]); }
pub struct Span { metric: usize, started: Instant }
pub fn span(metric: usize) -> Option<Span> {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    (*ENABLED.get_or_init(|| std::env::var("RESOURCE_PROFILE").as_deref() == Ok("detail")))
        .then(|| Span { metric, started: Instant::now() })
}
impl Drop for Span {
    fn drop(&mut self) {
        let nanos = self.started.elapsed().as_nanos();
        STATS.with(|s| { let mut s = s.borrow_mut(); s[self.metric].calls += 1; s[self.metric].nanos += nanos; });
    }
}
pub struct Report;
impl Drop for Report {
    fn drop(&mut self) {
        STATS.with(|s| {
            for (name, stat) in ["observed.revision", "observed.path_join", "observed.record_set"].iter().zip(s.borrow().iter()) {
                eprintln!("[resource-observed] {name} {} {}", stat.calls, stat.nanos);
            }
        });
    }
}
