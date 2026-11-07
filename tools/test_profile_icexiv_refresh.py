import shutil
import tempfile
from pathlib import Path
import unittest

from profile_icexiv_refresh import REPO, instrument, inventory


class RefreshTests(unittest.TestCase):
    def test_instruments_dirty_source_without_reordering_object_drops(self):
        for detailed in [False, True]:
            with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
                root = Path(directory)
                paths = list((REPO / 'src').rglob('*.rs')) + list((REPO / 'crates').rglob('*.rs'))
                for path in paths:
                    destination = root / path.relative_to(REPO)
                    destination.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(path, destination)
                original = (root / 'crates/notist-ssg/src/build.rs').read_text()
                instrument(root, detailed)
                result = (root / 'crates/notist-ssg/src/build.rs').read_text()
                self.assertIn('documents.into_iter().zip(handles)', result)
                self.assertEqual(result.count('drop(data);'), original.count('drop(data);'))
                production = result.split('#[cfg(test)]', 1)[0]
                self.assertNotIn('drop(documents)', production)
                self.assertNotIn('drop(handles)', production)
                self.assertIn('workers.clamp(1, 8)', result)
                head, body = result.split('pub(crate) fn build', 1)
                self.assertNotIn('xiv_profile', head)
                self.assertIn('measure("pipeline.evaluate", ||', body)
                for name in ['ssg.rs', 'publication.rs']:
                    self.assertNotIn('notist_core::', (root / 'crates/notist-cli/src' / name).read_text())
                evaluator = root / 'crates/notist-eval/src/eval.rs'
                if detailed and evaluator.exists():
                    self.assertIn('span("eval.execute")', evaluator.read_text())
                    self.assertIn('span("data.read")', (evaluator.parent / 'data.rs').read_text())

    def test_inventory_includes_hidden_data(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root = Path(directory)
            (root / '.data').mkdir()
            (root / '.data/one.json').write_text('[]')
            self.assertEqual(list(inventory(root)), ['.data/one.json'])


if __name__ == '__main__':
    unittest.main()
