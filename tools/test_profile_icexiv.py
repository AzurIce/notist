import tempfile
from pathlib import Path
import unittest

from profile_icexiv import inventory, instrument
from profile_xiv_history import REPO, extract, git


class IceXivProfileTests(unittest.TestCase):
    def test_inventory_tracks_hidden_data_but_not_python_cache(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            root = Path(directory)
            (root / ".data").mkdir()
            (root / ".data/input.json").write_text("[]")
            (root / "__pycache__").mkdir()
            (root / "__pycache__/ignore.pyc").write_bytes(b"cache")
            self.assertEqual(list(inventory(root)), [".data/input.json"])

    def test_instrumentation_preserves_inputs_and_worker_policy(self):
        for detailed in [False, True]:
            with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
                root = Path(directory) / "source"
                # Source-insertion probes belong to the frozen experiment's
                # revision; current SSG and kernel layouts are not that input.
                extract(REPO, "18f893a", root)
                lock = (root / "Cargo.lock").read_bytes()
                instrument(root, detailed)
                text = (root / "crates/notist-ssg/src/build.rs").read_text()
                self.assertIn("std::thread::available_parallelism()", text)
                self.assertIn("workers.clamp(1, 8)", text)
                self.assertIn('measure("pipeline.analyze", ||', text)
                head, body = text.split("pub(crate) fn build", 1)
                self.assertNotIn('measure("body.html"', head)
                if detailed:
                    self.assertIn('measure("body.html", || notist_html::Renderer', body)
                self.assertEqual((root / "Cargo.lock").read_bytes(), lock)
                self.assertIn("requests: 1_000_000", (root / "crates/notist-cli/src/ssg.rs").read_text())
                for relative in ["ssg.rs", "publication.rs"]:
                    self.assertNotIn("notist_core::", (root / "crates/notist-cli/src" / relative).read_text())


if __name__ == "__main__":
    unittest.main()
