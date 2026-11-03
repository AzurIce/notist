import json
import subprocess
import tempfile
import unittest
from pathlib import Path

from profile_optimizations import REPO, extract, git, instrument, write_visualization
from profile_publication_checks import instrument_publication


class OptimizationProfileTests(unittest.TestCase):
    def test_probe_targets_build_not_parallel_worker_bindings(self):
        original = (REPO / 'crates/notist-ssg/src/build.rs').read_bytes()
        with tempfile.TemporaryDirectory(dir='/tmp/opencode', prefix='optimized-probe-test-') as directory:
            source = Path(directory) / 'source'
            extract(REPO, '214de5f', source)
            patch = git(REPO, 'diff', '--binary', '214de5f', '--', 'src', 'crates', 'Cargo.toml', 'Cargo.lock')
            subprocess.run(['git', 'apply', '-'], cwd=source, input=patch, check=True)
            instrument(source)
            build = (source / 'crates/notist-ssg/src/build.rs').read_text()
            head, body = build.split('pub(crate) fn build', 1)
            self.assertNotIn('measure("body.html"', head)
            self.assertIn('measure("body.html"', body)
            self.assertIn('measure("publish.cleanup_retired"', (source / 'crates/notist-cli/src/publication.rs').read_text())
            publication = source / 'crates/notist-cli/src/publication.rs'
            publication.write_text(instrument_publication(publication.read_text()))
            self.assertIn('span("publish.path_validation")', publication.read_text())
            # Parsing is checked independently before a release build.
            subprocess.run(['rustfmt', '--emit', 'stdout', '--edition', '2024', '--config', 'skip_children=true',
                            str(source / 'crates/notist-ssg/src/build.rs')],
                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
            subprocess.run(['rustfmt', '--emit', 'stdout', '--edition', '2024', '--config', 'skip_children=true',
                            str(publication)], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
            out = Path(directory)
            samples = [{'label': 'multicore-0-' + mode, 'mode': mode, 'round': 0,
                        'wall_s': 1.0, 'user_s': 0.8, 'system_s': 0.2}
                       for mode in ['before', 'after']]
            write_visualization(out, {'rows': [], 'title': '</script>'}, {}, samples)
            html = (out / 'comparison.html').read_text()
            payload = html.split('<script id="profile-data" type="application/json">', 1)[1].split('</script>', 1)[0]
            self.assertEqual(json.loads(payload)['summary']['title'], '</script>')
            self.assertNotIn('__PROFILE_DATA__', html)
            self.assertIn('data-scenario="replace"', html)
        self.assertEqual((REPO / 'crates/notist-ssg/src/build.rs').read_bytes(), original)


if __name__ == '__main__':
    unittest.main()
