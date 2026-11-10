//! Optional, thread-local build observation. No terminal or IO dependencies.
//! Observers apply only to the current thread; worker tasks are reported by
//! their enclosing stage rather than implicitly inheriting host state.
use std::{borrow::Cow, cell::RefCell, rc::Rc, time::Duration};
use web_time::Instant;

pub mod timings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Started,
    Finished,
    Failed,
}

#[derive(Debug)]
pub struct Event<'a> {
    pub label: &'a str,
    pub depth: usize,
    pub status: Status,
    pub elapsed: Duration,
}

type Observer = Rc<dyn Fn(&Event<'_>)>;
thread_local! {
    static OBSERVER: RefCell<Option<Observer>> = RefCell::new(None);
    static DEPTH: RefCell<usize> = const { RefCell::new(0) };
}

/// Run with an observer, restoring the previous observer even on unwind.
pub fn observe<T>(observer: impl Fn(&Event<'_>) + 'static, f: impl FnOnce() -> T) -> T {
    struct Restore(Option<Observer>, usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            OBSERVER.with(|slot| *slot.borrow_mut() = self.0.take());
            DEPTH.with(|depth| *depth.borrow_mut() = self.1);
        }
    }
    let previous = OBSERVER.with(|slot| slot.replace(Some(Rc::new(observer))));
    let depth = DEPTH.with(|depth| depth.replace(0));
    let _restore = Restore(previous, depth);
    f()
}

/// A nested operation. Call `finish` on normal completion; early returns and
/// unwinding report failure. With no observer, this does not allocate labels.
pub struct Span {
    state: Option<(Observer, String, usize, Instant)>,
    timing: Option<timings::ActiveSpan>,
    finished: bool,
}
impl Span {
    pub fn enter(label: impl FnOnce() -> String) -> Self {
        Self::stage("operation", label)
    }

    /// Stable stage key, separate from its human-readable operation label.
    pub fn stage(stage: &'static str, label: impl FnOnce() -> String) -> Self {
        let observer = OBSERVER.with(|slot| slot.borrow().clone());
        let enabled = timings::enabled();
        let mut label = (observer.is_some() || enabled).then(label);
        let timing = if enabled {
            // The terminal observer needs its own label, but timings alone can
            // take ownership without allocating a second copy for every span.
            let label = if observer.is_some() {
                label.as_ref().unwrap().clone()
            } else {
                label.take().unwrap()
            };
            timings::enter(stage, label)
        } else {
            None
        };
        let state = observer.map(|observer| {
            let depth = DEPTH.with(|slot| {
                let depth = *slot.borrow();
                slot.replace(depth + 1);
                depth
            });
            let label = label.unwrap();
            observer(&Event {
                label: &label,
                depth,
                status: Status::Started,
                elapsed: Duration::ZERO,
            });
            (observer, label, depth, Instant::now())
        });
        Self {
            state,
            timing,
            finished: false,
        }
    }
    /// Attributes are allocated only while timing collection is enabled.
    pub fn attribute(&self, key: impl Into<Cow<'static, str>>, value: impl FnOnce() -> String) {
        if let Some(timing) = &self.timing {
            timing.attribute(key.into(), value());
        }
    }
    pub fn finish(mut self) {
        self.finished = true;
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(timing) = self.timing.take() {
            timing.finish(self.finished);
        }
        if let Some((observer, label, depth, started)) = &self.state {
            DEPTH.with(|slot| slot.replace(*depth));
            observer(&Event {
                label,
                depth: *depth,
                status: if self.finished {
                    Status::Finished
                } else {
                    Status::Failed
                },
                elapsed: started.elapsed(),
            });
        }
    }
}

pub fn run<T, E>(label: impl FnOnce() -> String, f: impl FnOnce() -> Result<T, E>) -> Result<T, E> {
    run_stage("operation", label, f)
}

pub fn run_stage<T, E>(
    stage: &'static str,
    label: impl FnOnce() -> String,
    f: impl FnOnce() -> Result<T, E>,
) -> Result<T, E> {
    let span = Span::stage(stage, label);
    let result = f();
    if result.is_ok() {
        span.finish();
    }
    result
}

/// Measure a normally returning operation, independently of its diagnostic result.
/// Unwinding is recorded as failure; callers can use an explicit Span for errors.
pub fn measure<T>(stage: &'static str, label: impl FnOnce() -> String, f: impl FnOnce() -> T) -> T {
    let span = Span::stage(stage, label);
    let result = f();
    span.finish();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unwind_and_nested_observers_restore_the_outer_scope() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        observe(
            move |e| captured.borrow_mut().push((e.depth, e.status)),
            || {
                let outer = Span::enter(|| "outer".into());
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    observe(
                        |_| {},
                        || {
                            let _inner = Span::enter(|| "inner".into());
                            panic!("abort inner observer");
                        },
                    );
                }));
                assert!(panic.is_err());
                let result: Result<(), ()> = run(|| "child".into(), || Ok(()));
                assert!(result.is_ok());
                // A worker does not accidentally inherit the host's observer.
                std::thread::spawn(|| Span::enter(|| panic!("no worker observer")).finish())
                    .join()
                    .unwrap();
                outer.finish();
            },
        );
        assert_eq!(
            *events.borrow(),
            vec![
                (0, Status::Started),
                (1, Status::Started),
                (1, Status::Finished),
                (0, Status::Finished),
            ]
        );
    }

    #[test]
    fn nested_errors_and_observer_restoration() {
        let events = Rc::new(RefCell::new(Vec::new()));
        let captured = events.clone();
        observe(
            move |e| {
                captured
                    .borrow_mut()
                    .push((e.label.to_owned(), e.depth, e.status))
            },
            || {
                let result: Result<(), ()> =
                    run(|| "target".into(), || run(|| "document".into(), || Err(())));
                assert!(result.is_err());
            },
        );
        assert_eq!(
            *events.borrow(),
            vec![
                ("target".into(), 0, Status::Started),
                ("document".into(), 1, Status::Started),
                ("document".into(), 1, Status::Failed),
                ("target".into(), 0, Status::Failed),
            ]
        );
        let span = Span::enter(|| panic!("disabled observation must not allocate labels"));
        span.finish();
    }
}
