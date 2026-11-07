import tempfile
from pathlib import Path
import unittest
import subprocess

from profile_boundaries import EVAL, REPO, evaluator_path, instrument, once


class BoundaryProfileTests(unittest.TestCase):
    def test_field_instrumentation_handles_both_recovery_representations(self):
        # This is a historical field/checkpoint experiment, not a current-kernel
        # instrumenter. Keep testing its original source contract explicitly.
        current = subprocess.check_output(["git", "show", "e5c65a3:" + EVAL], cwd=REPO, text=True)
        for owned in [False, True]:
            with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
                source = Path(directory)
                text = current if owned else once(current, "let mut boundary = Some(item);", "let boundary = item_shell(&item);")
                files = {
                    EVAL: text,
                    "crates/notist-core/src/lib.rs": "// core\n",
                    "src/lib.rs": "// facade\n",
                    "crates/notist-cli/src/main.rs": "fn main() -> ExitCode { ExitCode::SUCCESS }\n",
                }
                for relative, body in files.items():
                    path = source / relative
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(body)
                instrument(source)
                observed = (source / EVAL).read_text()
                self.assertEqual(observed.count("boundary_probe::field("), 7)
                self.assertIn("boundary_probe::site(1", observed)
                self.assertIn("boundary_probe::site(3", observed)
                self.assertEqual("boundary_probe::site(2" in observed, not owned)
                self.assertTrue((source / "crates/notist-core/src/boundary_probe.rs").is_file())

    def test_rewrites_fail_closed_when_the_source_shape_changes(self):
        with self.assertRaises(AssertionError):
            once("a a", "a", "b")
        with self.assertRaises(AssertionError):
            once("a", "missing", "b")

    def test_closure_kernel_is_rejected_before_any_snapshot_file_is_modified(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            source = Path(directory)
            path = source / EVAL
            path.parent.mkdir(parents=True)
            original = (REPO / EVAL).read_text()
            path.write_text(original)
            with self.assertRaisesRegex(ValueError, "closure kernel"):
                instrument(source)
            self.assertEqual(path.read_text(), original)

    def test_snapshot_layout_not_live_checkout_selects_the_evaluator(self):
        for relative in ["crates/notist-pipeline/src/code/eval.rs", "crates/notist-eval/src/eval.rs"]:
            with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
                source = Path(directory)
                path = source / relative
                path.parent.mkdir(parents=True)
                path.write_text("// frozen evaluator\n")
                self.assertEqual(evaluator_path(source), relative)


if __name__ == "__main__":
    unittest.main()
