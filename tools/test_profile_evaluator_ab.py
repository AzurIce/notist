import tempfile
from pathlib import Path
import unittest

from profile_evaluator_ab import KINDS, observer, variant
from profile_xiv_history import REPO, extract


class EvaluatorABTests(unittest.TestCase):
    def test_independent_variants_keep_limits_and_the_input_tree_contract(self):
        for kind in KINDS:
            with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
                root = Path(directory) / "source"
                extract(REPO, "85ab69e", root)
                lock = (root / "Cargo.lock").read_bytes()
                variant(root, kind)
                text = (root / "crates/notist-pipeline/src/code/eval.rs").read_text()
                self.assertIn("evaluation_reads_the_supplied_item_tree_without_a_parallel_representation", text)
                self.assertIn("self.limits.call_depth.min(32)", text)
                self.assertIn("self.tick()?", text)
                self.assertEqual((root / "Cargo.lock").read_bytes(), lock)
                if kind in ["instance", "combined"]:
                    self.assertIn("implementation_cache: Vec::new(),\n                        program: analysis.program.clone()", text)

    def test_observer_is_a_separate_copy_and_keeps_automatic_drop_order(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            root = Path(directory) / "source"
            extract(REPO, "85ab69e", root)
            observer(root)
            text = (root / "crates/notist-ssg/src/build.rs").read_text()
            self.assertIn("eval_probe::page(&route.source)", text)
            self.assertNotIn("drop(documents)", text)
            self.assertIn("std::thread::available_parallelism()", text)


if __name__ == "__main__":
    unittest.main()
