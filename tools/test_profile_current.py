"""Tests for the committed-state profiler; no production source mutation."""
from pathlib import Path
import tempfile
import unittest

from profile_current import PHASES, detailed_instrument, parse_detail, partitions
from profile_evaluator import instrument_evaluator
from profile_xiv_history import REPO, extract


class CurrentProfileTests(unittest.TestCase):
    def test_partitions_are_disjoint_and_account_for_the_whole_cli(self):
        metrics = {name: {"seconds": 1.0} for name in PHASES}
        metrics.update({name: {"seconds": value} for name, value in {
            "pipeline.analyze": .3, "pipeline.evaluate": .4,
            "ssg.total": 10, "cli.publish": 2, "cli.total": 15,
        }.items()})
        result = partitions(metrics)
        self.assertAlmostEqual(sum(result.values()), 15)
        self.assertEqual(result["site_teardown"], 1)
        self.assertAlmostEqual(result["pipeline_other"], .3)

    def test_detail_parser_preserves_self_times_counts_edges_and_units(self):
        text = '''[current-time] cli.total 1 3.000000000 1.000000000 3.000000000
[current-time] work 2 2.000000000 2.000000000 1.500000000
[current-time] input.source_hit 521 0 0 0
[current-edge] cli.total work 2 2.000000000 2.000000000
[xiv-module] compile_work 18 0.010000000
'''
        metrics, edges, modules = parse_detail(text)
        self.assertEqual(metrics["input.source_hit"]["calls"], 521)
        self.assertEqual(metrics["work"]["self_seconds"], 2)
        self.assertEqual(edges[0]["parent"], "cli.total")
        self.assertEqual(modules["compile_work"]["calls"], 18)
        with self.assertRaises(AssertionError):
            parse_detail(text.replace("2.000000000 2.000000000 1.500000000", "2.000000000 3.000000000 1.500000000"))

    def test_instrumentation_only_changes_its_disposable_snapshot(self):
        before = (REPO / "src/resource_session.rs").read_bytes()
        with tempfile.TemporaryDirectory(dir="/tmp/opencode", prefix="current-profile-test-") as temp:
            root = Path(temp) / "source"
            extract(REPO, "214de5f", root)
            detailed_instrument(root)
            session = (root / "src/resource_session.rs").read_text()
            self.assertIn('count("input.source_hit", 1)', session)
            self.assertNotIn("fn validate(", session)
            self.assertNotIn("RESOURCE_DIAGNOSTIC", session)
            self.assertIn('span("io.bounded_read")', (root / "src/resources.rs").read_text())
            self.assertNotIn("notist_core::current_profile", (root / "crates/notist-ssg/src/assets.rs").read_text())
            self.assertIn('measure("publish.cleanup_retired", || drop(backup))', (root / "crates/notist-cli/src/publication.rs").read_text())
            instrument_evaluator(root)
            evaluator = (root / "crates/notist-pipeline/src/code/eval.rs").read_text()
            self.assertIn('measure("eval.clone_item", || item.clone())', evaluator)
            self.assertIn('measure("eval.clone_body", || record.body.clone())', evaluator)
            self.assertIn('count.saturating_add(added) > limit', evaluator)
            self.assertIn('self.steps > self.limits.steps', evaluator)
        self.assertEqual((REPO / "src/resource_session.rs").read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
