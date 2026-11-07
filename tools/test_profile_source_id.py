import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest

import profile_source_id as profile


class SourceIdProfileTests(unittest.TestCase):
    def fixture(self, directory):
        out = Path(directory)
        for name in ["reproducer", "sources/before", "sources/after", "site", "bin", "logs"]:
            (out / name).mkdir(parents=True)
        (out / "site/input.txt").write_text("unchanged")
        binary = out / "bin/before"
        binary.write_text("#!/usr/bin/env python3\nfrom pathlib import Path\np=Path('dist/icexiv-db');p.mkdir(parents=True,exist_ok=True)\n(p/'index.html').write_text('complete output')\n")
        binary.chmod(0o755)
        manifest = dict(inputs=profile.kernel.inventory(out / "site"), input_symlinks={},
                        sources={k: {} for k in ["before", "after"]},
                        binaries={"before": profile.digest(binary)}, reproducer={})
        (out / "manifest.json").write_text(json.dumps(manifest))
        expected = out / "expected"
        expected.mkdir()
        (expected / "index.html").write_text("complete output")
        (out / "output-inventory.json").write_text(json.dumps(profile.kernel.inventory(expected)))
        return SimpleNamespace(out=out)

    def test_guard_rejects_changed_frozen_inputs(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            args = self.fixture(directory)
            profile.guard(args)
            (args.out / "site/input.txt").write_text("tampered")
            with self.assertRaises(AssertionError):
                profile.guard(args)

    def test_clean_process_verifies_complete_output_without_touching_inputs(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            args = self.fixture(directory)
            row = profile.run_once(args, "before", "test")
            self.assertFalse(row["diagnostic"])
            self.assertTrue(row["complete_output_identical"])
            self.assertEqual(row["exit"], 0)
            profile.guard(args)

    def test_process_rejects_output_mismatch(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            args = self.fixture(directory)
            (args.out / "output-inventory.json").write_text("{}")
            with self.assertRaises(AssertionError):
                profile.run_once(args, "before", "mismatch")


if __name__ == "__main__":
    unittest.main()
