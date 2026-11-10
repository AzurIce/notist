//! Per-operation timing sessions. Worker contexts are propagated explicitly.
use serde::{Serialize, Serializer, ser::SerializeMap};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::BTreeMap,
    io::{self, Write},
    marker::PhantomData,
    ops::Index,
    rc::Rc,
    sync::{Arc, Mutex},
};
use web_time::Instant;

#[derive(Clone, Debug, Serialize)]
pub struct Record {
    pub id: usize,
    pub parent: Option<usize>,
    pub stage: &'static str,
    pub label: String,
    pub lane: usize,
    pub start_ns: u64,
    pub end_ns: u64,
    pub status: &'static str,
    pub attributes: Attributes,
}

/// A small, sorted attribute map. Most spans have zero or one attribute, so a
/// tree node per populated span is wasteful. Static keys need no allocation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Attributes(Vec<(Cow<'static, str>, String)>);

impl Attributes {
    pub fn insert(&mut self, key: impl Into<Cow<'static, str>>, value: String) -> Option<String> {
        let key = key.into();
        match self.0.binary_search_by(|(k, _)| k.as_ref().cmp(&key)) {
            Ok(index) => Some(std::mem::replace(&mut self.0[index].1, value)),
            Err(index) => {
                if self.0.is_empty() {
                    self.0.reserve_exact(1);
                }
                self.0.insert(index, (key, value));
                None
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&String> {
        self.0
            .binary_search_by(|(k, _)| k.as_ref().cmp(key))
            .ok()
            .map(|index| &self.0[index].1)
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &String)> {
        self.0.iter().map(|(key, value)| (key.as_ref(), value))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Index<&str> for Attributes {
    type Output = String;

    fn index(&self, key: &str) -> &Self::Output {
        self.get(key).expect("missing timing attribute")
    }
}

impl Serialize for Attributes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (key, value) in self.iter() {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
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
        let mut result = BTreeMap::<&'static str, Measurement>::new();
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
            let entry = result.entry(span.stage).or_default();
            entry.calls += 1;
            entry.inclusive_ns += span.end_ns - span.start_ns;
            entry.self_ns += subtract_children(span, children.remove(&span.id).unwrap_or_default());
        }
        result
            .into_iter()
            .map(|(stage, measurement)| (stage.into(), measurement))
            .collect()
    }

    pub fn json(&self) -> String {
        serde_json::to_string(self).expect("timing report is serializable")
    }

    /// Stream a report without allocating the complete JSON string.
    pub fn write_json(&self, writer: impl Write) -> serde_json::Result<()> {
        serde_json::to_writer(writer, self)
    }

    /// Standalone, offline report. JSON is escaped before embedding in HTML.
    pub fn html(&self) -> String {
        let mut bytes = Vec::new();
        self.write_html(&mut bytes)
            .expect("writing to a Vec cannot fail");
        String::from_utf8(bytes).expect("timing HTML is UTF-8")
    }

    /// Stream HTML, escaping JSON as it is written rather than making several
    /// complete copies of a potentially very large report.
    pub fn write_html(&self, mut writer: impl Write) -> io::Result<()> {
        let (before, after) = include_str!("timings.html")
            .split_once("/*REPORT*/null")
            .expect("timing HTML contains the report placeholder");
        writer.write_all(before.as_bytes())?;
        self.write_json(HtmlJson(&mut writer))
            .map_err(io::Error::from)?;
        writer.write_all(after.as_bytes())
    }
}

struct HtmlJson<W>(W);

impl<W: Write> Write for HtmlJson<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut start = 0;
        for (index, byte) in bytes.iter().enumerate() {
            let escaped: &[u8] = match byte {
                b'&' => br"\u0026",
                b'<' => br"\u003c",
                b'>' => br"\u003e",
                _ => continue,
            };
            self.0.write_all(&bytes[start..index])?;
            self.0.write_all(escaped)?;
            start = index + 1;
        }
        self.0.write_all(&bytes[start..])?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
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
    // Taking this Vec closes the session and transfers ownership to the report.
    records: Mutex<Option<Vec<Record>>>,
    lanes: Mutex<Vec<std::thread::ThreadId>>,
}

#[derive(Clone)]
pub struct Context {
    session: Arc<Session>,
    parent: Option<usize>,
    lane: usize,
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
        let state = context().and_then(|context| {
            let start = context.session.records.lock().unwrap().as_ref()?.len();
            Some((context, start))
        });
        Self { state, work_stage }
    }

    pub fn outcome(&self) -> &'static str {
        let Some((context, start)) = &self.state else {
            return "unobserved";
        };
        let records = context.session.records.lock().unwrap();
        let Some(records) = records.as_ref() else {
            return "unobserved";
        };
        if records[*start..]
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
pub fn with_context<T>(mut context: Option<Context>, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Context>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CURRENT.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    // Resolve the lane on context installation, not on every nested span.
    // Re-resolving matters when a context is propagated to a worker thread.
    if let Some(context) = &mut context {
        let thread = std::thread::current().id();
        let mut lanes = context.session.lanes.lock().unwrap();
        context.lane = match lanes.iter().position(|id| *id == thread) {
            Some(lane) => lane,
            None => {
                lanes.push(thread);
                lanes.len() - 1
            }
        };
    }
    let _restore = Restore(CURRENT.with(|slot| slot.replace(context)));
    f()
}

/// All explicitly propagated worker tasks must finish before this returns.
/// Collection composes with the thread-local terminal progress observer.
pub fn capture<T>(f: impl FnOnce() -> T) -> (T, Report) {
    let session = Arc::new(Session {
        start: Instant::now(),
        records: Mutex::new(Some(Vec::new())),
        lanes: Mutex::default(),
    });
    let result = with_context(
        Some(Context {
            session: session.clone(),
            parent: None,
            lane: 0,
        }),
        f,
    );
    let report = Report {
        elapsed_ns: nanos(session.start),
        attributes: BTreeMap::new(),
        spans: session.records.lock().unwrap().take().unwrap(),
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
    context: Option<Context>,
    id: usize,
    success: bool,
    // A scope must be closed on its entering thread.
    _local: PhantomData<Rc<()>>,
}

pub(crate) fn enter(stage: &'static str, label: String) -> Option<ActiveSpan> {
    let context = context()?;
    let start_ns = nanos(context.session.start);
    let id = {
        let mut records = context.session.records.lock().unwrap();
        let records = records.as_mut()?;
        let id = records.len();
        records.push(Record {
            id,
            parent: context.parent,
            stage,
            label,
            lane: context.lane,
            start_ns,
            end_ns: start_ns,
            status: "running",
            attributes: Attributes::default(),
        });
        id
    };
    CURRENT.with(|slot| {
        slot.borrow_mut().as_mut().unwrap().parent = Some(id);
    });
    Some(ActiveSpan {
        context: Some(context),
        id,
        success: false,
        _local: PhantomData,
    })
}

impl ActiveSpan {
    pub(crate) fn attribute(&self, key: Cow<'static, str>, value: String) {
        let context = self.context.as_ref().unwrap();
        if let Some(records) = context.session.records.lock().unwrap().as_mut() {
            records[self.id].attributes.insert(key, value);
        }
    }

    pub(crate) fn finish(mut self, success: bool) {
        self.success = success;
    }
}

impl Drop for ActiveSpan {
    fn drop(&mut self) {
        let context = self.context.take().unwrap();
        let end_ns = nanos(context.session.start);
        {
            let mut records = context.session.records.lock().unwrap();
            let Some(records) = records.as_mut() else {
                return;
            };
            let record = &mut records[self.id];
            record.end_ns = end_ns;
            record.status = if self.success { "finished" } else { "failed" };
        }
        CURRENT.with(|slot| *slot.borrow_mut() = Some(context));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Span;

    #[test]
    fn attributes_remain_a_sorted_json_object() {
        let mut attributes = Attributes::default();
        assert!(attributes.is_empty());
        assert_eq!(attributes.insert("source", "one.not".into()), None);
        assert_eq!(attributes.0.capacity(), 1);
        assert!(matches!(attributes.0[0].0, Cow::Borrowed(_)));
        attributes.insert(String::from("cache"), "miss".into());
        assert_eq!(
            attributes.insert("source", "two.not".into()),
            Some("one.not".into())
        );
        assert_eq!(attributes.len(), 2);
        assert_eq!(attributes.get("missing"), None);
        assert_eq!(attributes["cache"], "miss");
        assert_eq!(attributes.clone(), attributes);
        assert_eq!(
            serde_json::to_string(&attributes).unwrap(),
            r#"{"cache":"miss","source":"two.not"}"#
        );
        assert_eq!(serde_json::to_string(&Attributes::default()).unwrap(), "{}");
    }

    #[test]
    fn capture_transfers_the_record_buffer_and_label_allocation() {
        let ((buffer, label), report) = capture(|| {
            let label = String::from("A dynamically allocated label");
            let pointer = label.as_ptr();
            Span::stage("test.move", || label).finish();
            let context = context().unwrap();
            let records = context.session.records.lock().unwrap();
            (records.as_ref().unwrap().as_ptr(), pointer)
        });
        assert_eq!(report.spans.as_ptr(), buffer);
        assert_eq!(report.spans[0].label.as_ptr(), label);
        assert_eq!(report.spans[0].stage, "test.move");
    }

    #[test]
    fn observer_and_timings_keep_the_same_label_and_attributes() {
        let labels = Rc::new(RefCell::new(Vec::new()));
        let seen = labels.clone();
        let (_, report) = crate::observe(
            move |event| seen.borrow_mut().push(event.label.to_string()),
            || {
                capture(|| {
                    let span = Span::stage("test.observe", || "Shared label".into());
                    span.attribute("source", || "one.not".into());
                    span.attribute(String::from("owned key"), || "value".into());
                    span.finish();
                })
            },
        );
        assert_eq!(&*labels.borrow(), &["Shared label", "Shared label"]);
        assert_eq!(report.spans[0].label, "Shared label");
        assert_eq!(report.spans[0].attributes["owned key"], "value");
    }

    #[test]
    fn closed_contexts_and_escaped_guards_cannot_append_or_leak() {
        let ((span, saved), report) = capture(|| {
            let span = Span::stage("test.escaped", || "Escaped".into());
            (span, context())
        });
        assert_eq!(report.spans.len(), 1);
        assert_eq!(report.spans[0].status, "running");
        span.attribute("late", || "ignored".into());
        span.finish();
        assert!(!enabled());
        with_context(saved, || {
            Span::stage("test.late", || "Late".into()).finish()
        });
        assert!(!enabled());
        assert_eq!(report.spans.len(), 1);
        assert!(report.spans[0].attributes.is_empty());
    }

    #[test]
    fn lanes_survive_repeated_context_installations_and_nested_capture() {
        let (_, report) = capture(|| {
            let root = Span::stage("root", || "Root".into());
            let worker = context();
            std::thread::spawn(move || {
                for _ in 0..2 {
                    with_context(worker.clone(), || {
                        Span::stage("worker", || "Worker".into()).finish();
                    });
                }
            })
            .join()
            .unwrap();
            let (_, inner) = capture(|| Span::stage("inner", || "Inner".into()).finish());
            assert_eq!(inner.spans[0].lane, 0);
            Span::stage("after", || "After nested capture".into()).finish();
            root.finish();
        });
        assert_eq!(report.spans[1].lane, report.spans[2].lane);
        assert_ne!(report.spans[0].lane, report.spans[1].lane);
        assert_eq!(report.spans[0].lane, report.spans[3].lane);
        assert!(report.spans[1..].iter().all(|span| span.parent == Some(0)));
    }

    #[test]
    fn streaming_exports_preserve_json_and_html_escaping() {
        struct ShortWriter(Vec<u8>);
        impl Write for ShortWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                let count = bytes.len().min(2);
                self.0.extend_from_slice(&bytes[..count]);
                Ok(count)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let (_, report) = capture(|| {
            let span = Span::stage("test.export", || "中文 & </script> <> \\\"".into());
            span.attribute("source", || "<&中文>.not".into());
            span.finish();
        });
        let mut json = ShortWriter(Vec::new());
        report.write_json(&mut json).unwrap();
        assert_eq!(json.0, report.json().as_bytes());
        let expected = include_str!("timings.html").replace(
            "/*REPORT*/null",
            &report
                .json()
                .replace('&', "\\u0026")
                .replace('<', "\\u003c")
                .replace('>', "\\u003e"),
        );
        let mut html = ShortWriter(Vec::new());
        report.write_html(&mut html).unwrap();
        assert_eq!(html.0, expected.as_bytes());
        assert_eq!(report.html(), expected);
    }

    #[test]
    fn streaming_exports_propagate_write_errors() {
        struct FailingWriter(usize);
        impl Write for FailingWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.0 == 0 {
                    return Err(io::ErrorKind::BrokenPipe.into());
                }
                let count = bytes.len().min(self.0);
                self.0 -= count;
                Ok(count)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let report = Report::default();
        assert_eq!(
            report
                .write_json(FailingWriter(5))
                .unwrap_err()
                .io_error_kind(),
            Some(io::ErrorKind::BrokenPipe)
        );
        let prefix = include_str!("timings.html")
            .split_once("/*REPORT*/null")
            .unwrap()
            .0
            .len();
        assert_eq!(
            report
                .write_html(FailingWriter(prefix + 5))
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }

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
            stage: "stage",
            label: "label".into(),
            lane: 0,
            status: "finished",
            attributes: Attributes::default(),
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
