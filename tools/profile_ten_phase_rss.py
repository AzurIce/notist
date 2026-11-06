#!/usr/bin/env python3
"""Separate stage RSS/HWM evidence without a large tree observer HashSet."""
import argparse

import profile_runtime_cow as common
import profile_ten_probe as probe
from profile_ten_rounds import OUT


def run(name):
    probe.LABEL = name + '-phases'
    probe.prepare(name)
    source = OUT / 'sources' / ('probe-' + probe.LABEL)
    helper = source / 'crates/notist-ssg/src/tree_residency_probe.rs'
    helper.write_text('''
use std::sync::Mutex;
static POINTS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());
pub(super) fn observe(_: &[super::Document]) { capture("evaluation"); }
pub(super) fn capture(label: &str) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let get = |key: &str| -> u64 {
        status.lines().find_map(|line| line.strip_prefix(key))
            .unwrap().split_whitespace().next().unwrap().parse::<u64>().unwrap() * 1024
    };
    POINTS.lock().unwrap().push(serde_json::json!({"phase": label,
        "rss_bytes": get("VmRSS:"), "hwm_bytes": get("VmHWM:")}));
}
pub(super) fn report() {
    eprintln!("[tree-residency] {}", serde_json::json!({"points": *POINTS.lock().unwrap(),
        "note": "Diagnostic stage boundary RSS and cumulative kernel HWM, not an exclusive per-stage peak or a primary gain sample. No full-tree HashSet observer."}));
}
''')
    path = source / 'crates/notist-ssg/src/build.rs'
    text = path.read_text()
    for variable, label in [('analysis_stage', 'analysis'), ('asset_stage', 'assets'),
                            ('rendering', 'html'), ('templates_stage', 'templates')]:
        anchor = f'    {variable}.finish();\n'
        assert text.count(anchor) == 1
        extra = f'    tree_residency_probe::capture("{label}");\n'
        if variable == 'templates_stage':
            extra += '    tree_residency_probe::report();\n'
        text = text.replace(anchor, anchor + extra)
    path.write_text(text)
    common.save(OUT / ('probe-' + probe.LABEL + '-sources.json'), common.inventory(source))
    probe.build(name)
    probe.run(name)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--name', default='r04')
    run(parser.parse_args().name)
