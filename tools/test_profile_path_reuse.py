from pathlib import Path
from types import SimpleNamespace
import json
import tempfile
import unittest
from unittest.mock import patch

import profile_path_reuse as profile


class PathReuseProfileTests(unittest.TestCase):
    def test_prepare_never_overwrites_existing_directory(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            with self.assertRaises(ValueError):
                profile.prepare(SimpleNamespace(out=Path(directory)))

    def test_prepare_rejects_different_base_before_creating_output(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root = Path(directory)
            reference = root / 'reference'
            reference.mkdir()
            (reference / 'manifest.json').write_text(json.dumps({'revision': 'old'}))
            args = SimpleNamespace(out=root / 'evidence', reference=reference)
            with patch.object(profile.subprocess, 'check_output', return_value='new\n'):
                with self.assertRaises(ValueError):
                    profile.prepare(args)
            self.assertFalse(args.out.exists())

    def test_measure_never_overwrites_samples(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root = Path(directory)
            (root / 'clean-samples.json').write_text('[]')
            with self.assertRaises(ValueError):
                profile.measure(SimpleNamespace(out=root))

    def test_verify_rejects_modified_executable(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root = Path(directory)
            (root / 'sources/after').mkdir(parents=True)
            (root / 'bin').mkdir()
            binary = root / 'bin/before'
            binary.write_bytes(b'before')
            manifest = dict(sources={'after': {}}, binaries={'before': profile.digest(binary)})
            (root / 'manifest.json').write_text(json.dumps(manifest))
            binary.write_bytes(b'changed')
            with self.assertRaises(AssertionError):
                profile.verify(SimpleNamespace(out=root))


if __name__ == '__main__':
    unittest.main()
