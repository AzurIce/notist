import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import profile_source_identity as profile


class SourceIdentityProfileTests(unittest.TestCase):
    def test_summary_keeps_clean_measurements_separate_and_uses_medians(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            target = Path(directory) / "summary.json"
            rows = [dict(kind=kind, wall_s=value) for kind, value in [
                ("before", 10), ("before", 100), ("before", 12),
                ("after", 9), ("after", 10), ("after", 11),
            ]]
            profile.summarize(target, rows, ["wall_s"])
            summary = json.loads(target.read_text())
            self.assertEqual(summary["before"]["wall_s"], 12)
            self.assertEqual(summary["after"]["wall_s"], 10)
            self.assertAlmostEqual(summary["change_percent"]["wall_s"], -100 / 6)

    def test_prepare_rejects_existing_directories_before_touching_them(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            sentinel = Path(directory) / "keep.txt"
            sentinel.write_text("keep")
            args = type("Args", (), dict(out=Path(directory)))()
            with self.assertRaises(ValueError):
                profile.prepare(args)
            self.assertEqual(sentinel.read_text(), "keep")

    def test_clean_runs_cannot_accidentally_use_allocation_instrumentation(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            root = Path(directory)
            (root / "bin").mkdir()
            binary = root / "bin/before"
            binary.write_bytes(b"binary")
            profile.save(root / "manifest.json", dict(binaries=dict(before=profile.digest(binary))))
            args = type("Args", (), dict(out=root))()
            with patch.object(profile, "environment", return_value=dict(LD_PRELOAD="observer.so")):
                with self.assertRaisesRegex(ValueError, "LD_PRELOAD"):
                    profile.run_once(args, "before", "clean")

    def test_build_cannot_invalidate_existing_measurements(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            root = Path(directory)
            (root / "samples.json").write_text("[]")
            args = type("Args", (), dict(out=root))()
            with self.assertRaisesRegex(ValueError, "existing measurements"):
                profile.build(args)


if __name__ == "__main__":
    unittest.main()
