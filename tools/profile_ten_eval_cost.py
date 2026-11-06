#!/usr/bin/env python3
"""Isolated inclusive evaluator cost probe; never a performance gain sample."""
import argparse
import profile_runtime_cow as common
import profile_ten_probe as probe
from profile_ten_rounds import OUT


def run(name):
    probe.LABEL = 'eval-cost-' + name
    probe.prepare(name)
    source = OUT / 'sources' / ('probe-' + probe.LABEL)
    path = source / 'crates/notist-pipeline/src/code/eval.rs'
    text = path.read_text()
    anchors = [
        '    fn synthetic_call(&self, item: &Item) -> Option<Invocation> {\n',
        '    fn present(&mut self, forest: Vec<Item>, limit: usize) -> (Vec<Item>, ContentSummary) {\n',
        '    ) -> Result<Runtime, Failure> {\n        self.tick()?;\n',
        '    fn expr(&mut self, t: &TypedExpr, scope: &Scope) -> Result<Runtime, Failure> {\n',
    ]
    for index, anchor in enumerate(anchors):
        assert text.count(anchor) == 1, anchor
        # Execute's anchor includes its tick so the guard still starts at entry.
        replacement = anchor.replace('        self.tick()?;\n', '')
        replacement += f'        let _cost = eval_cost_probe::Guard::new({index});\n'
        if 'self.tick()' in anchor:
            replacement += '        self.tick()?;\n'
        text = text.replace(anchor, replacement)
    text += '''
pub(crate) fn evaluator_cost_probe_report() { eval_cost_probe::report(); }
mod eval_cost_probe {
    use std::{sync::atomic::{AtomicU64, Ordering::Relaxed}, time::Instant};
    static CALLS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
    static NANOS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
    pub(super) struct Guard { index: usize, start: Instant }
    impl Guard {
        pub(super) fn new(index: usize) -> Self { Self { index, start: Instant::now() } }
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            CALLS[self.index].fetch_add(1, Relaxed);
            NANOS[self.index].fetch_add(self.start.elapsed().as_nanos() as u64, Relaxed);
        }
    }
    pub(super) fn report() {
        for (index, label) in ["synthetic_call", "present_recursive", "execute", "expr_recursive"].iter().enumerate() {
            eprintln!("[eval-cost] {} calls={} inclusive_ns={}", label, CALLS[index].load(Relaxed), NANOS[index].load(Relaxed));
        }
    }
}
'''
    path.write_text(text)
    path = source / 'crates/notist-pipeline/src/lib.rs'
    path.write_text(path.read_text() + '\npub fn evaluator_cost_probe_report() { code::eval::evaluator_cost_probe_report(); }\n')
    path = source / 'src/lib.rs'
    path.write_text(path.read_text() + '\npub use notist_pipeline::evaluator_cost_probe_report;\n')
    path = source / 'crates/notist-ssg/src/build.rs'
    text = path.read_text()
    anchor = '    evaluation_stage.finish();\n'
    assert text.count(anchor) == 1
    path.write_text(text.replace(anchor, anchor + '    notist::evaluator_cost_probe_report();\n'))
    common.save(OUT / ('probe-' + probe.LABEL + '-sources.json'), common.inventory(source))
    probe.build(name)
    probe.run(name)
    lines = (OUT / 'logs' / ('probe-' + probe.LABEL + '.stderr')).read_text().splitlines()
    rows = [line for line in lines if line.startswith('[eval-cost] ')]
    assert len(rows) == 4
    common.save(OUT / (probe.LABEL + '.json'), dict(rows=rows,
        note='Inclusive instrumented wall costs with recursive nesting and timer/atomic overhead. Never sum, subtract as exclusive cost, or use for ordinary speed/RSS acceptance.'))
    print('\n'.join(rows), flush=True)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--name', default='r04')
    run(parser.parse_args().name)
