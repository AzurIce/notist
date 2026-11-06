import tempfile
from pathlib import Path
import unittest

from profile_icexiv import instrument as coarse_instrument
from profile_icexiv_teardown import instrument
from profile_xiv_history import REPO, extract


class TeardownProfileTests(unittest.TestCase):
    def test_field_drops_follow_struct_declaration_order(self):
        with tempfile.TemporaryDirectory(dir='/tmp/opencode') as directory:
            root = Path(directory) / 'source'
            extract(REPO, '85ab69e', root)
            coarse_instrument(root, False)
            instrument(root)
            for relative, prefix, fields in [
                ('src/vault.rs', 'vault.drop.', ['resources', 'pipeline', 'extensions', 'environments', 'environment_cache', 'identity', 'config', 'module_urls', 'evaluation_limits']),
                ('src/resource_session.rs', 'session.drop.', ['resources', 'roots', 'limits', 'state', 'data_cache']),
                ('src/resource_session.rs', 'session.state.drop.', ['files', 'kinds', 'sources', 'texts', 'dependencies', 'bytes', 'requests']),
            ]:
                text = (root / relative).read_text()
                offsets = [text.index('"' + prefix + name + '"') for name in fields]
                self.assertEqual(offsets, sorted(offsets))
            self.assertIn('|| vault.profile_drop()', (root / 'crates/notist-cli/src/ssg.rs').read_text())


if __name__ == '__main__':
    unittest.main()
