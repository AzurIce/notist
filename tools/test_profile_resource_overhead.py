"""Regression guards for the disposable resource attribution probes."""
from pathlib import Path
import tempfile
import unittest

from profile_resource_overhead import MODES, chart_payload, fine_instrument, fingerprints, measurement_cells, parse_resource, parse_strace, totals
from profile_xiv_history import REPO, git


class ResourceProfileTests(unittest.TestCase):
    def test_measurements_separate_normal_and_diagnostic_binaries(self):
        cells = measurement_cells()
        self.assertEqual(len(cells), 43)
        self.assertEqual(len(cells), len(set(cells)))
        self.assertEqual(len([c for c in cells if c[2] == "normal"]), 15)
        for workload in ["literal", "private"]:
            self.assertEqual({mode for engine, w, mode in cells if engine == "current" and w == workload and mode != "normal"}, set(MODES))

    def test_parsing_preserves_zero_counts_phases_and_units(self):
        text = "\n".join(f"[resource-profile] {phase} validate.total {i} {i * 1000000000} {i+3}" for i, phase in enumerate(["other", "sources", "analyze", "evaluate"]))
        sample = {"resource": parse_resource(text)}
        self.assertEqual(totals(sample, "validate.total"), {"calls": 6, "seconds": 6.0, "max": 6})
        self.assertEqual(sample["resource"]["other"]["validate.total"]["calls"], 0)
        with self.assertRaises(AssertionError):
            parse_resource("[resource-profile] other validate.total 0 0 0")

    def test_instrumentation_is_confined_to_a_copy_and_keeps_local_checks(self):
        paths = ["src/lib.rs", "src/resource_session.rs", "src/vault.rs", "src/resources.rs", "crates/notist-core/src/lib.rs",
                 "crates/notist-core/src/data/cache.rs", "crates/notist-core/src/data/mod.rs", "crates/notist-core/src/data/parse.rs",
                 "crates/notist-pipeline/src/code/data.rs", "crates/notist-cli/src/main.rs"]
        with tempfile.TemporaryDirectory(dir="/tmp/opencode", prefix="resource-profile-unit-") as directory:
            root = Path(directory)
            before = {rel: (REPO / rel).read_bytes() for rel in paths}
            # This probe targets the archived pre-snapshot implementation, not
            # whichever resource model the working checkout currently uses.
            for rel in paths:
                (root / rel).parent.mkdir(parents=True, exist_ok=True)
                (root / rel).write_bytes(git(REPO, "show", f"083ff6c:{rel}"))
            fine_instrument(root)
            session = (root / "src/resource_session.rs").read_text()
            self.assertIn("before != reader.revision(&relative)", session)
            self.assertIn("let (root, relative) = match self.grant(&absolute)", session)
            self.assertIn("state.changed\n                || (files", session)
            self.assertLess(session.index("self.request().and_then(|_| self.validate())"), session.index("Metric::SourceHit"))
            self.assertEqual({rel: (REPO / rel).read_bytes() for rel in paths}, before)

    def test_fingerprints_exclude_generated_outputs_only_when_requested(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode", prefix="resource-profile-unit-") as directory:
            root = Path(directory)
            (root / "input.not").write_text("hello")
            (root / "dist").mkdir()
            (root / "dist/index.html").write_text("output")
            self.assertEqual(set(fingerprints(root, exclude_dist=True)), {"input.not"})
            self.assertEqual(set(fingerprints(root)), {"input.not", "dist/index.html"})

    def test_strace_summary_handles_optional_error_counts(self):
        result = parse_strace('''% time     seconds  usecs/call     calls    errors syscall
 70.00    0.070000           7     10000        12 openat
 30.00    0.030000           5      6000           close
100.00    0.100000           6     16000        12 total
''')
        self.assertEqual(result["openat"], {"calls": 10000, "errors": 12})
        self.assertEqual(result["close"], {"calls": 6000, "errors": 0})
        self.assertEqual(result["total"]["calls"], 16000)

    def test_chart_does_not_mix_normal_rankings_with_probe_attribution(self):
        resource = {"other": {"validate.total": {"calls": 1, "seconds": 2.0, "max": 0}, "validate.file_checks": {"calls": 100, "seconds": 0.0, "max": 0}}}
        probe = {"engine": "current", "workload": "literal", "detail": False, "wall_s": 3.0, "resource": resource}
        cells = [{"engine": "current", "workload": "literal", "mode": mode, "wall_s": {"median": 1.0}} for mode in ["normal", "full", "no-scans"]]
        data = chart_payload({"runs": 3, "probes": [probe, dict(probe, detail=True)], "cells": cells})
        self.assertEqual(len(data["probes"]), 1)
        self.assertEqual(data["probes"][0]["checks"], 100)
        self.assertEqual({c["mode"] for c in data["cells"]}, {"full", "no-scans"})
        self.assertEqual(data["builds"], 12)


if __name__ == "__main__":
    unittest.main()
