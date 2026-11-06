import tempfile
from pathlib import Path
import unittest

from profile_boundaries import EVAL, REPO, instrument, once


class BoundaryProfileTests(unittest.TestCase):
    def test_field_instrumentation_handles_both_recovery_representations(self):
        current = (REPO / EVAL).read_text()
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


if __name__ == "__main__":
    unittest.main()
