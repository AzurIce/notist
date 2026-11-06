import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest.mock import patch

import profile_ten_rounds as campaign
from profile_ten_validate import observed_frames


class TenRoundContract(unittest.TestCase):
    def test_iteration_limit_and_inflight_guard(self):
        with tempfile.TemporaryDirectory() as tmp:
            manifest = dict(max_experiments=10, experiments=[], binaries={'r00': 'frozen'})
            args = types.SimpleNamespace(name='r01', control='r00', hypothesis='one change')
            with patch.object(campaign, 'OUT', Path(tmp)), patch.object(campaign, 'guard', lambda: manifest):
                campaign.begin(args)
                self.assertEqual(len(manifest['experiments']), 1)
                with self.assertRaises(AssertionError):
                    campaign.begin(types.SimpleNamespace(name='r02', control='r00', hypothesis='second'))
                manifest['experiments'] = [dict(state='rejected') for _ in range(10)]
                with self.assertRaises(AssertionError):
                    campaign.begin(types.SimpleNamespace(name='r11', control='r00', hypothesis='eleventh'))

    def test_dependency_frames_keep_complete_ordered_payload(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / 'trace'
            payload = [['/project/引号".not', '/project/.data/0.json'], [{'root': '/project', 'path': '.data/0.json'}]]
            path.write_text('ordinary diagnostic\n[semantic-page] ["observed","build"]\n[semantic-json] ' + json.dumps(payload) + '\n')
            self.assertEqual(observed_frames(path), [payload])


if __name__ == '__main__':
    unittest.main()
