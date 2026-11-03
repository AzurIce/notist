//! Disposable std-only timing sink. Never linked into production builds.
use std::{cell::RefCell, collections::BTreeMap, time::Instant};

#[derive(Clone, Copy, Debug, Default)]
pub struct Metric {
    pub calls: u64,
    pub nanos: u128,
    pub self_nanos: u128,
    pub maximum: u128,
}
struct Frame {
    name: &'static str,
    children: u128,
}
#[derive(Default)]
struct State {
    stack: Vec<Frame>,
    metrics: BTreeMap<&'static str, Metric>,
    edges: BTreeMap<(&'static str, &'static str), Metric>,
}
thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}
pub struct Span {
    name: &'static str,
    depth: usize,
    started: Instant,
}
pub fn span(name: &'static str) -> Span {
    let started = Instant::now();
    let depth = STATE.with(|state| {
        let mut state = state.borrow_mut();
        let depth = state.stack.len();
        state.stack.push(Frame { name, children: 0 });
        depth
    });
    Span {
        name,
        depth,
        started,
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        let nanos = self.started.elapsed().as_nanos();
        STATE.with(|state| {
            let mut state = state.borrow_mut();
            let frame = state.stack.pop().expect("timing stack underflow");
            assert_eq!(state.stack.len(), self.depth, "non-nested timing spans");
            assert_eq!(frame.name, self.name);
            let own = nanos
                .checked_sub(frame.children)
                .expect("child exceeds parent");
            let parent = if let Some(parent) = state.stack.last_mut() {
                parent.children += nanos;
                parent.name
            } else {
                "<root>"
            };
            let metric = state.metrics.entry(self.name).or_default();
            metric.calls += 1;
            metric.nanos += nanos;
            metric.self_nanos += own;
            metric.maximum = metric.maximum.max(nanos);
            let edge = state.edges.entry((parent, self.name)).or_default();
            edge.calls += 1;
            edge.nanos += nanos;
            edge.self_nanos += own;
            edge.maximum = edge.maximum.max(nanos);
        });
    }
}
pub fn measure<T>(name: &'static str, run: impl FnOnce() -> T) -> T {
    let _span = span(name);
    run()
}
pub fn count(name: &'static str, amount: usize) {
    STATE.with(|state| state.borrow_mut().metrics.entry(name).or_default().calls += amount as u64);
}
pub fn report() {
    STATE.with(|state| {
        let state = state.borrow();
        assert!(state.stack.is_empty(), "unfinished timing spans");
        for (name, m) in &state.metrics {
            eprintln!(
                "[current-time] {name} {} {:.9} {:.9} {:.9}",
                m.calls,
                m.nanos as f64 / 1e9,
                m.self_nanos as f64 / 1e9,
                m.maximum as f64 / 1e9
            );
        }
        for ((parent, child), m) in &state.edges {
            eprintln!(
                "[current-edge] {parent} {child} {} {:.9} {:.9}",
                m.calls,
                m.nanos as f64 / 1e9,
                m.self_nanos as f64 / 1e9
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn self_times_partition_the_root_even_for_recursive_names() {
        {
            let _root = span("root");
            measure("child", || {
                let _recursive = span("child");
                std::hint::black_box((0..100).sum::<usize>());
            });
            count("counter", 3);
        }
        STATE.with(|state| {
            let state = state.borrow();
            assert!(state.stack.is_empty());
            assert_eq!(state.metrics["child"].calls, 2);
            assert_eq!(state.metrics["counter"].calls, 3);
            assert_eq!(
                state.metrics.values().map(|m| m.self_nanos).sum::<u128>(),
                state.metrics["root"].nanos
            );
            assert_eq!(state.edges[&("child", "child")].calls, 1);
        });
    }
    #[test]
    fn early_results_close_their_timing_scope() {
        let result: Result<(), ()> = measure("fallible", || Err(()));
        assert!(result.is_err());
        STATE.with(|state| assert!(state.borrow().stack.is_empty()));
    }
}
