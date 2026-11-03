//! Diagnostic-only instrumentation, copied into disposable source snapshots.
//! Never linked into production binaries by this repository.
use std::{cell::{Cell, RefCell}, sync::OnceLock, time::Instant};

macro_rules! metrics {
    ($($name:ident => $label:literal),+ $(,)?) => {
        #[derive(Clone, Copy)]
        #[repr(usize)]
        pub enum Metric { $($name,)+ Count }
        const NAMES: [&str; Metric::Count as usize] = [$($label,)+];
    }
}
metrics! {
    Validate => "validate.total",
    ValidateFiles => "validate.files",
    ValidateKinds => "validate.kinds",
    FileChecks => "validate.file_checks",
    KindChecks => "validate.kind_checks",
    ObservedFiles => "observed.files_max",
    ObservedKinds => "observed.kinds_max",
    SourceBytes => "session.source_bytes",
    SourceHit => "session.source_hit",
    SourceMiss => "session.source_miss",
    Request => "session.request",
    Load => "session.load",
    LoadHit => "session.load_hit",
    LoadMiss => "session.load_miss",
    DataRead => "session.data_read",
    ObserveSource => "session.observe_source",
    FsRevision => "fs.revision",
    FsOpen => "fs.cap_open",
    FsReadBounded => "fs.read_bounded",
    FsKind => "fs.kind",
    FsReadRoot => "fs.read_root",
    FsRawRead => "fs.raw_read",
    DataNative => "data.native",
    DataReadNative => "data.read_native",
    DataParse => "data.parse_actual",
    DataDecode => "data.decode_actual",
    CacheParse => "data.cache_parse",
    CacheDecode => "data.cache_decode",
    CacheParseHit => "data.cache_parse_hit",
    CacheParseMiss => "data.cache_parse_miss",
}

#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Phase { Other, Sources, Analyze, Evaluate }
const PHASES: [&str; 4] = ["other", "sources", "analyze", "evaluate"];
#[derive(Clone, Copy)]
#[repr(usize)]
pub enum Cause { Other, Source, Data }
#[derive(Clone, Copy, Default)]
struct Stat { calls: u64, nanos: u128, maximum: u64 }
thread_local! {
    static STATS: RefCell<[[Stat; NAMES.len()]; PHASES.len()]> =
        RefCell::new([[Stat::default(); NAMES.len()]; PHASES.len()]);
    static PHASE: Cell<usize> = const { Cell::new(Phase::Other as usize) };
    static CAUSE: Cell<usize> = const { Cell::new(Cause::Other as usize) };
}
struct Settings { enabled: bool, detail: bool, mode: String }
fn settings() -> &'static Settings {
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        let probe = std::env::var("RESOURCE_PROFILE").unwrap_or_default();
        let mode = std::env::var("RESOURCE_DIAGNOSTIC").unwrap_or_else(|_| "full".into());
        assert!(["full", "no-files", "no-kinds", "no-source-scans", "no-eval-scans", "no-scans"].contains(&mode.as_str()));
        Settings { enabled: probe == "1" || probe == "detail", detail: probe == "detail", mode }
    })
}
pub struct PhaseGuard(usize);
pub fn phase(value: Phase) -> PhaseGuard {
    PhaseGuard(PHASE.with(|p| p.replace(value as usize)))
}
impl Drop for PhaseGuard { fn drop(&mut self) { PHASE.with(|p| p.set(self.0)); } }
pub struct CauseGuard(usize);
pub fn cause(value: Cause) -> CauseGuard {
    CauseGuard(CAUSE.with(|p| p.replace(value as usize)))
}
impl Drop for CauseGuard { fn drop(&mut self) { CAUSE.with(|p| p.set(self.0)); } }

pub struct Span { metric: usize, phase: usize, started: Instant }
pub fn span(metric: Metric) -> Option<Span> {
    settings().enabled.then(|| Span { metric: metric as usize, phase: PHASE.with(Cell::get), started: Instant::now() })
}
// High-frequency filesystem calls are counted without a timer by default.
// Detailed runs time these too, and are never used for ordinary CLI ranking.
pub fn detail(metric: Metric) -> Option<Span> {
    if settings().detail { span(metric) } else { count(metric, 1); None }
}
impl Drop for Span {
    fn drop(&mut self) {
        let nanos = self.started.elapsed().as_nanos();
        STATS.with(|s| {
            let mut s = s.borrow_mut();
            let m = &mut s[self.phase][self.metric];
            m.calls += 1;
            m.nanos += nanos;
        });
    }
}
pub fn count(metric: Metric, calls: u64) {
    if settings().enabled {
        let phase = PHASE.with(Cell::get);
        STATS.with(|s| s.borrow_mut()[phase][metric as usize].calls += calls);
    }
}
pub fn maximum(metric: Metric, value: usize) {
    if settings().enabled {
        let phase = PHASE.with(Cell::get);
        STATS.with(|s| {
            let mut s = s.borrow_mut();
            let m = &mut s[phase][metric as usize];
            m.maximum = m.maximum.max(value as u64);
        });
    }
}
// UNSAFE diagnostic ablations, not alternative input policies. Local grant,
// request limits, load-before/after revisions and sticky failures remain, but
// these modes intentionally cannot guarantee batch consistency under edits.
pub fn scan_policy() -> (bool, bool) {
    let mode = settings().mode.as_str();
    let skip = mode == "no-scans"
        || (mode == "no-source-scans" && CAUSE.with(Cell::get) == Cause::Source as usize)
        || (mode == "no-eval-scans" && PHASE.with(Cell::get) == Phase::Evaluate as usize
            && CAUSE.with(Cell::get) == Cause::Other as usize);
    (!skip && mode != "no-files", !skip && mode != "no-kinds")
}
pub struct Report;
impl Drop for Report {
    fn drop(&mut self) {
        if !settings().enabled { return; }
        eprintln!("[resource-config] {} {}", settings().mode, settings().detail);
        STATS.with(|s| {
            for (p, row) in s.borrow().iter().enumerate() {
                for (i, m) in row.iter().enumerate() {
                    eprintln!("[resource-profile] {} {} {} {} {}", PHASES[p], NAMES[i], m.calls, m.nanos, m.maximum);
                }
            }
        });
    }
}
