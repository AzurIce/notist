import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest

import profile_definition_identity as profile


class DefinitionIdentityProfileTests(unittest.TestCase):
    def test_prepare_never_overwrites_an_existing_directory(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root = Path(directory)
            (root / 'keep').write_text('user data')
            with self.assertRaises(ValueError):
                profile.prepare(SimpleNamespace(out=root))
            self.assertEqual((root / 'keep').read_text(), 'user data')

    def fixture(self, root):
        for name in ['sources/before', 'sources/after', 'bin', 'site']:
            (root / name).mkdir(parents=True)
        (root / 'sources/before/code.rs').write_text('before')
        (root / 'sources/after/code.rs').write_text('after')
        (root / 'site/input').write_text('fixed input')
        (root / 'bin/before').write_text('frozen executable')
        manifest = dict(
            sources={kind: profile.inventory(root / 'sources' / kind) for kind in ['before', 'after']},
            inputs=profile.inventory(root / 'site'),
            binaries={'before': profile.digest(root / 'bin/before')},
        )
        (root / 'manifest.json').write_text(json.dumps(manifest))
        return SimpleNamespace(out=root)

    def test_verify_rejects_changed_sources_and_inputs(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            args = self.fixture(Path(directory))
            profile.verify(args)
            (args.out / 'sources/after/code.rs').write_text('tampered')
            with self.assertRaises(AssertionError):
                profile.verify(args)
            (args.out / 'sources/after/code.rs').write_text('after')
            (args.out / 'site/input').write_text('tampered input')
            with self.assertRaises(AssertionError):
                profile.verify(args)

    def test_build_cannot_replace_a_recorded_executable(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            args = self.fixture(Path(directory))
            with self.assertRaises(ValueError):
                profile.build_kind(args, 'before')


if __name__ == '__main__':
    unittest.main()
