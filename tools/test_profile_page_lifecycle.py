import tempfile
from pathlib import Path
from types import SimpleNamespace
import unittest

import profile_page_lifecycle as profile


class LifecycleProfileTests(unittest.TestCase):
    def test_medians_retain_every_sample(self):
        rows = [{k: n for k in ['wall_s', 'cpu_s', 'user_s', 'system_s', 'max_rss_kib']}
                for n in [1, 99, 3]]
        self.assertEqual(profile.summarize(rows)['wall_s'], 3)

    def test_prepare_never_overwrites_an_evidence_directory(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(ValueError):
                profile.prepare(SimpleNamespace(out=Path(root)))

    def test_run_never_overwrites_a_measurement(self):
        with tempfile.TemporaryDirectory() as root:
            out = Path(root)
            (out / 'logs').mkdir()
            (out / 'logs/test.usage.json').write_text('{}')
            with self.assertRaises(ValueError):
                profile.run(SimpleNamespace(out=out), 'after', 'test', 4)


if __name__ == '__main__':
    unittest.main()
