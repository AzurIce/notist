//! Per-operation timing sessions. Worker contexts are propagated explicitly.
use serde::Serialize;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, Mutex},
};
use web_time::Instant;

#[derive(Clone, Debug, Serialize)]
pub struct Record {
    pub id: usize,
    pub parent: Option<usize>,
    pub stage: String,
    pub label: String,
    pub lane: usize,
    pub start_ns: u64,
    pub end_ns: u64,
    pub status: &'static str,
    pub attributes: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub elapsed_ns: u64,
    pub attributes: BTreeMap<String, String>,
    pub spans: Vec<Record>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Measurement {
    pub calls: usize,
    pub inclusive_ns: u64,
    pub self_ns: u64,
}

impl Report {
    /// Subtract the union of direct child intervals, clipped to the parent.
    pub fn self_ns(&self, span: &Record) -> u64 {
        let intervals: Vec<_> = self
            .spans
            .iter()
            .filter(|child| child.parent == Some(span.id))
            .map(|child| {
                (
                    child.start_ns.max(span.start_ns),
                    child.end_ns.min(span.end_ns),
                )
            })
            .filter(|(start, end)| end > start)
            .collect();
        subtract_children(span, intervals)
    }

    pub fn summary(&self) -> BTreeMap<String, Measurement> {
        let mut result = BTreeMap::<String, Measurement>::new();
        let mut children = BTreeMap::<usize, Vec<(u64, u64)>>::new();
        for span in &self.spans {
            if let Some(parent) = span.parent {
                children
                    .entry(parent)
                    .or_default()
                    .push((span.start_ns, span.end_ns));
            }
        }
        for span in &self.spans {
            let entry = result.entry(span.stage.clone()).or_default();
            entry.calls += 1;
            entry.inclusive_ns += span.end_ns - span.start_ns;
            entry.self_ns += subtract_children(span, children.remove(&span.id).unwrap_or_default());
        }
        result
    }

    pub fn json(&self) -> String {
        serde_json::to_string(self).expect("timing report is serializable")
    }

    /// Standalone, offline report. JSON is escaped before embedding in HTML.
    pub fn html(&self) -> String {
        let data = self
            .json()
            .replace('&', "\\u0026")
            .replace('<', "\\u003c")
            .replace('>', "\\u003e");
        include_str!("timings.html").replace("/*REPORT*/null", &data)
    }
}

fn subtract_children(span: &Record, intervals: Vec<(u64, u64)>) -> u64 {
    let mut intervals: Vec<_> = intervals
        .into_iter()
        .map(|(start, end)| (start.max(span.start_ns), end.min(span.end_ns)))
        .filter(|(start, end)| end > start)
        .collect();
    intervals.sort_unstable();
    let mut covered = 0;
    let mut edge = span.start_ns;
    for (start, end) in intervals {
        covered += end.saturating_sub(start.max(edge));
        edge = edge.max(end);
    }
    (span.end_ns - span.start_ns).saturating_sub(covered)
}

struct Session {
    start: Instant,
    records: Mutex<Vec<Record>>,
    lanes: Mutex<Vec<std::thread::ThreadId>>,
}

#[derive(Clone)]
pub struct Context {
    session: Arc<Session>,
    parent: Option<usize>,
}

thread_local! {
    static CURRENT: RefCell<Option<Context>> = const { RefCell::new(None) };
}

/// Capture the current parent before spawning a worker. No implicit inheritance.
pub fn context() -> Option<Context> {
    CURRENT.with(|slot| slot.borrow().clone())
}

/// Detect whether a memoized call executed its direct child work stage.
/// The probe does no clock reads or allocations when collection is disabled.
pub struct CacheProbe {
    state: Option<(Context, usize)>,
    work_stage: &'static str,
}

impl CacheProbe {
    pub fn new(work_stage: &'static str) -> Self {
        let state = context().map(|context| {
            let start = context.session.records.lock().unwrap().len();
            (context, start)
        });
        Self { state, work_stage }
    }

    pub fn outcome(&self) -> &'static str {
        let Some((context, start)) = &self.state else {
            return "unobserved";
        };
        if context.session.records.lock().unwrap()[*start..]
            .iter()
            .any(|record| record.parent == context.parent && record.stage == self.work_stage)
        {
            "miss"
        } else {
            "hit"
        }
    }
}

/// Install a worker context, restoring any previous session even on unwind.
pub fn with_context<T>(context: Option<Context>, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Context>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(CURRENT.with(|slot| slot.replace(context)));
    f()
}

/// All explicitly propagated worker tasks must finish before this returns.
/// Collection composes with the thread-local terminal progress observer.
pub fn capture<T>(f: impl FnOnce() -> T) -> (T, Report) {
    let session = Arc::new(Session {
        start: Instant::now(),
        records: Mutex::default(),
        lanes: Mutex::default(),
    });
    let result = with_context(
        Some(Context {
            session: session.clone(),
            parent: None,
        }),
        f,
    );
    let report = Report {
        elapsed_ns: nanos(session.start),
        attributes: BTreeMap::new(),
        spans: session.records.lock().unwrap().clone(),
    };
    (result, report)
}

fn nanos(start: Instant) -> u64 {
    start.elapsed().as_nanos().min(u64::MAX as u128) as u64
}

pub(crate) fn enabled() -> bool {
    CURRENT.with(|slot| slot.borrow().is_some())
}

pub(crate) struct ActiveSpan {
    context: Context,
    id: usize,
    success: bool,
    // A scope must be closed on its entering thread.
    _local: PhantomData<Rc<()>>,
}

pub(crate) fn enter(stage: &str, label: &str) -> Option<ActiveSpan> {
    let context = context()?;
    let start_ns = nanos(context.session.start);
    let lane = {
        let thread = std::thread::current().id();
        let mut lanes = context.session.lanes.lock().unwrap();
        match lanes.iter().position(|id| *id == thread) {
            Some(lane) => lane,
            None => {
                lanes.push(thread);
                lanes.len() - 1
            }
        }
    };
    let id = {
        let mut records = context.session.records.lock().unwrap();
        let id = records.len();
        records.push(Record {
            id,
            parent: context.parent,
            stage: stage.into(),
            label: label.into(),
            lane,
            start_ns,
            end_ns: start_ns,
            status: "running",
            attributes: BTreeMap::new(),
        });
        id
    };
    CURRENT.with(|slot| {
        *slot.borrow_mut() = Some(Context {
            session: context.session.clone(),
            parent: Some(id),
        })
    });
    Some(ActiveSpan {
        context,
        id,
        success: false,
        _local: PhantomData,
    })
}

impl ActiveSpan {
    pub(crate) fn attribute(&self, key: &str, value: String) {
        self.context.session.records.lock().unwrap()[self.id]
            .attributes
            .insert(key.into(), value);
    }

    pub(crate) fn finish(mut self, success: bool) {
        self.success = success;
    }
}

impl Drop for ActiveSpan {
    fn drop(&mut self) {
        let end_ns = nanos(self.context.session.start);
        let mut records = self.context.session.records.lock().unwrap();
        let record = &mut records[self.id];
        record.end_ns = end_ns;
        record.status = if self.success { "finished" } else { "failed" };
        drop(records);
        CURRENT.with(|slot| *slot.borrow_mut() = Some(self.context.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Span;

    #[test]
    fn workers_and_nested_capture_restore_context() {
        let (_, report) = capture(|| {
            let root = Span::stage("build", || "build".into());
            let context = context();
            std::thread::spawn(move || {
                with_context(context, || {
                    Span::stage("template", || "page".into()).finish();
                })
            })
            .join()
            .unwrap();
            let (_, inner) = capture(|| Span::stage("inner", || "inner".into()).finish());
            assert_eq!(inner.spans[0].parent, None);
            root.finish();
        });
        assert_eq!(report.spans.len(), 2);
        assert_eq!(report.spans[1].parent, Some(0));
        assert_ne!(report.spans[0].lane, report.spans[1].lane);
        assert!(!enabled());
    }

    #[test]
    fn failure_and_disabled_labels() {
        Span::stage("disabled", || panic!("must stay lazy")).finish();
        let (_, report) = capture(|| {
            let _span = Span::stage("fail", || "</script>".into());
        });
        assert_eq!(report.spans[0].status, "failed");
        assert!(!report.html().contains("\"</script>\""));
        let _ = std::panic::catch_unwind(|| capture(|| panic!("test")));
        assert!(!enabled());
    }

    #[test]
    fn child_union_is_clipped_and_not_double_counted() {
        let record = |id, parent, start_ns, end_ns| Record {
            id,
            parent,
            start_ns,
            end_ns,
            stage: "stage".into(),
            label: "label".into(),
            lane: 0,
            status: "finished",
            attributes: BTreeMap::new(),
        };
        let report = Report {
            elapsed_ns: 100,
            attributes: BTreeMap::new(),
            spans: vec![
                record(0, None, 10, 100),
                record(1, Some(0), 20, 60),
                record(2, Some(0), 40, 80),
                record(3, Some(0), 90, 120),
            ],
        };
        assert_eq!(report.self_ns(&report.spans[0]), 20);
    }
}
