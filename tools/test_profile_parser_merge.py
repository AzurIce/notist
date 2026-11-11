import json
from pathlib import Path
import tempfile
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import profile_parser_merge as profile


class ComparisonTests(unittest.TestCase):
    def test_stage_time_distinguishes_overlapping_worker_intervals(self):
        report = {'spans': [
            {'stage': 'parse', 'start_ns': 0, 'end_ns': 2_000_000_000},
            {'stage': 'parse', 'start_ns': 1_000_000_000, 'end_ns': 3_000_000_000},
            {'stage': 'parse', 'start_ns': 4_000_000_000, 'end_ns': 5_000_000_000},
        ]}
        self.assertEqual(profile.timing_summary(report), {
            'parse': {'calls': 3, 'inclusive_s': 5, 'interval_union_s': 4},
        })

    def test_run_checks_complete_outputs_outside_timing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for part in ['bin', 'site', 'logs']:
                (root / part).mkdir()
            binary = root / 'bin/before-release'
            binary.write_text(
                f'#!{sys.executable}\n'
                'from pathlib import Path\n'
                'p = Path("notist-output/icexiv-db/index.html")\n'
                'p.parent.mkdir(parents=True, exist_ok=True)\n'
                'p.write_text("page")\n'
            )
            binary.chmod(0o755)
            args = SimpleNamespace(out=root)
            row = profile.run(args, 'before', 'first')
            self.assertEqual(row['files'], 1)
            self.assertGreater(row['max_rss_kib'], 0)
            expected = json.loads((root / 'output-inventory.json').read_text())
            self.assertEqual(expected, profile.inventory(root / 'site/notist-output/icexiv-db'))
            binary.write_text(binary.read_text().replace('"page"', '"changed"'))
            with self.assertRaisesRegex(AssertionError, 'output mismatch'):
                profile.run(args, 'before', 'changed')

    def test_rounds_are_interleaved_and_warmups_excluded(self):
        with tempfile.TemporaryDirectory() as directory:
            args = SimpleNamespace(out=Path(directory), rounds=3)
            calls = []

            def run(args, kind, label):
                calls.append((kind, label))
                value = 100 if 'warmup' in label else (4 if kind == 'before' else 2)
                return dict(kind=kind, label=label, wall_s=value, user_s=value,
                            system_s=value, max_rss_kib=value)

            with patch.object(profile, 'run', side_effect=run):
                profile.measure(args)
            self.assertEqual([kind for kind, _ in calls],
                             ['before', 'after', 'before', 'after',
                              'after', 'before', 'before', 'after'])
            summary = json.loads((args.out / 'summary.json').read_text())
            self.assertEqual(summary['wall_speedup'], 2)
            self.assertEqual(summary['before']['wall_s'], 4)

    def test_measure_does_not_overwrite_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            args = SimpleNamespace(out=Path(directory), rounds=1)
            (args.out / 'samples.json').write_text('[]')
            with self.assertRaises(ValueError):
                profile.measure(args)

    def test_prepare_requires_fresh_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaises(ValueError):
                profile.prepare(SimpleNamespace(out=Path(directory)))


if __name__ == '__main__':
    unittest.main()
