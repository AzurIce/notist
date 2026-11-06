from pathlib import Path
import tempfile
import unittest

from profile_boundaries import EVAL, REPO
from profile_source_text import FILES, instrument


class SourceTextProfileTests(unittest.TestCase):
    def test_expand_is_a_separate_allocation_scope_from_origin_clone(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            source = Path(directory)
            for relative, text in {
                EVAL: (REPO / EVAL).read_text(),
                "crates/notist-core/src/lib.rs": "// core\n",
                "src/lib.rs": "// facade\n",
                "crates/notist-cli/src/main.rs": "fn main() -> ExitCode { ExitCode::SUCCESS }\n",
            }.items():
                path = source / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            instrument(source)
            observed = (source / EVAL).read_text()
            self.assertEqual(observed.count("boundary_probe::field("), 10)
            self.assertIn("field(4, || origin.expansion.extend_from_slice(locations))", observed)
            probe = (source / "crates/notist-core/src/boundary_probe.rs").read_text()
            self.assertIn('"expand"', probe)
            self.assertIn('"checkpoint"', probe)
            self.assertNotIn('"present_recovery"', probe)

    def test_overlay_whitelist_excludes_dependencies_and_consumer_inputs(self):
        self.assertEqual(len(FILES), len(set(FILES)))
        self.assertTrue(all(path.endswith(".rs") for path in FILES))
        self.assertIn("crates/notist-core/src/item.rs", FILES)
        self.assertIn("src/preview.rs", FILES)
        self.assertNotIn("Cargo.lock", FILES)


if __name__ == "__main__":
    unittest.main()
