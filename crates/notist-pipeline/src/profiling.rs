//! Opt-in, thread-local module stage measurements. Disabled in normal builds.
use std::{
    cell::{Cell, RefCell},
    time::Instant,
};

/// Profiling-only controls for reproducing the three stages in one binary.
#[derive(Debug, Clone, Copy, Default)]
pub enum CacheMode {
    Baseline,
    CompileOnly,
    #[default]
    Both,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Measurement {
    pub calls: u64,
    pub nanos: u128,
}

#[derive(Debug, Clone, Default)]
pub struct Stats {
    pub compile: Measurement,
    pub compile_work: Measurement,
    pub bind: Measurement,
    pub bind_work: Measurement,
    pub link: Measurement,
    pub install: Measurement,
}

thread_local! {
    static STATS: RefCell<Stats> = RefCell::default();
    static MODE: Cell<CacheMode> = Cell::new(CacheMode::Both);
}

pub fn set_cache_mode(mode: CacheMode) {
    MODE.with(|current| current.set(mode));
}

pub(crate) fn compile_enabled() -> bool {
    MODE.with(|mode| !matches!(mode.get(), CacheMode::Baseline))
}

pub(crate) fn binding_enabled() -> bool {
    MODE.with(|mode| matches!(mode.get(), CacheMode::Both))
}

/// Return and reset measurements for the calling thread. Times are inclusive:
/// compile_work is part of compile, and bind is part of link.
pub fn take() -> Stats {
    STATS.with(|stats| std::mem::take(&mut *stats.borrow_mut()))
}

#[derive(Clone, Copy)]
pub(crate) enum Stage {
    Compile,
    CompileWork,
    Bind,
    BindWork,
    Link,
    Install,
}

pub(crate) struct Timer(Stage, Instant);

pub(crate) fn enter(stage: Stage) -> Timer {
    Timer(stage, Instant::now())
}

impl Drop for Timer {
    fn drop(&mut self) {
        let elapsed = self.1.elapsed().as_nanos();
        STATS.with(|stats| {
            let mut stats = stats.borrow_mut();
            let target = match self.0 {
                Stage::Compile => &mut stats.compile,
                Stage::CompileWork => &mut stats.compile_work,
                Stage::Bind => &mut stats.bind,
                Stage::BindWork => &mut stats.bind_work,
                Stage::Link => &mut stats.link,
                Stage::Install => &mut stats.install,
            };
            target.calls += 1;
            target.nanos += elapsed;
        });
    }
}
