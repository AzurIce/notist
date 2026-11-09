from pathlib import Path
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import patch

import profile_binding_identity as profile
import profile_definition_identity as shared


class BindingIdentityProfileTests(unittest.TestCase):
    def test_prepare_never_overwrites_existing_evidence(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root = Path(directory)
            (root / 'keep').write_text('retained evidence')
            with self.assertRaises(ValueError):
                profile.prepare(SimpleNamespace(out=root))
            self.assertEqual((root / 'keep').read_text(), 'retained evidence')

    def test_build_rebuilds_both_frozen_revisions(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root = Path(directory)
            (root / 'manifest.json').write_text('{"binaries":{"before":"old","after":"new"}}')
            args = SimpleNamespace(out=root)
            with patch.object(profile, 'build_kind') as build:
                profile.build(args)
            self.assertEqual([call.args for call in build.call_args_list], [(args, 'before'), (args, 'after')])
            (root / 'manifest.json').write_text('{"binaries":{"before":"same","after":"same"}}')
            with patch.object(profile, 'build_kind'), self.assertRaises(ValueError):
                profile.build(args)

    def test_clean_and_probe_builds_use_distinct_target_directories(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root = Path(directory)
            (root / 'logs').mkdir()
            args = SimpleNamespace(out=root, target=root / 'target')
            for kind in ['before', 'after', 'before-ledger', 'after-ledger']:
                with patch.object(shared, 'verify', return_value={'binaries': {}}), patch.object(shared.subprocess, 'run') as run, patch.object(shared.shutil, 'copy2') as copy, patch.object(shared, 'digest', return_value=kind), patch.object(shared, 'save'):
                    shared.build_kind(args, kind)
                self.assertEqual(run.call_args.kwargs['env']['CARGO_TARGET_DIR'], str(args.target / kind))
                self.assertEqual(copy.call_args.args[0], args.target / kind / 'release/notist')
