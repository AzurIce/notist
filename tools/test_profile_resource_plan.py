import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest

import profile_resource_plan as profile


class ResourcePlanProfileTests(unittest.TestCase):
    def test_fresh_evidence_directory_is_required(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                profile.prepare(SimpleNamespace(out=Path(directory)))

    def test_snapshot_scope_does_not_include_unrelated_work(self):
        self.assertEqual(len(profile.SCOPED), len(set(profile.SCOPED)))
        self.assertNotIn('docs/SUMMARY.not', profile.SCOPED)
        self.assertTrue(all(not Path(name).is_absolute() and '..' not in Path(name).parts
                            for name in profile.SCOPED))
        self.assertTrue(all((profile.REPO / name).is_file() for name in profile.SCOPED))


if __name__ == '__main__':
    unittest.main()
