from pathlib import Path
import tempfile
import unittest

from profile_boundaries import EVAL, REPO
from profile_present_checkpoint import instrument


class PresentCheckpointProbeTests(unittest.TestCase):
    def test_both_old_shell_and_new_checkpoint_fields_are_observed(self):
        current = (REPO / EVAL).read_text()
        for checkpoint in [False, True]:
            with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
                source = Path(directory)
                body = current if checkpoint else current.replace("fn before_call(", "fn unrelated_call(", 1)
                for relative, text in {
                    EVAL: body,
                    "crates/notist-core/src/lib.rs": "// core\n",
                    "src/lib.rs": "// facade\n",
                    "crates/notist-cli/src/main.rs": "fn main() -> ExitCode { ExitCode::SUCCESS }\n",
                }.items():
                    path = source / relative
                    path.parent.mkdir(parents=True, exist_ok=True)
                    path.write_text(text)
                instrument(source)
                observed = (source / EVAL).read_text()
                self.assertEqual(observed.count("boundary_probe::field("), 9 if checkpoint else 7)
                probe = (source / "crates/notist-core/src/boundary_probe.rs").read_text()
                self.assertIn('"checkpoint"', probe)
                self.assertNotIn('"call_recovery"', probe)
                self.assertIn('#[cfg(any(test, feature = "test-support"))]\n    fn present_reference(', observed)


if __name__ == "__main__":
    unittest.main()
