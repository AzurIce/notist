import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from profile_optimizations import REPO, extract, git, instrument
from profile_residuals import allocator_events, checker_probe, evaluator_probe, io_probe


class ResidualProbeTests(unittest.TestCase):
    def test_separate_disposable_probes_preserve_production_and_parse(self):
        original = (REPO / 'crates/notist-pipeline/src/code/eval.rs').read_bytes()
        with tempfile.TemporaryDirectory(dir='/tmp/opencode', prefix='residual-probe-test-') as directory:
            directory = Path(directory)
            template = directory / 'template'
            extract(REPO, '214de5f', template)
            patch = git(REPO, 'diff', '--binary', '214de5f', '--', 'src', 'crates', 'Cargo.toml', 'Cargo.lock')
            subprocess.run(['git', 'apply', '-'], cwd=template, input=patch, check=True)
            instrument(template)
            for mode, rewrite, files in [
                ('io', io_probe, ['src/resources.rs', 'crates/notist-cli/src/publication.rs']),
                ('evaluator', evaluator_probe, ['src/environment/loader.rs', 'crates/notist-pipeline/src/code/eval.rs']),
                ('checker', checker_probe, ['crates/notist-pipeline/src/definitions.rs',
                     'crates/notist-pipeline/src/code/check.rs', 'crates/notist-pipeline/src/context.rs',
                     'src/environment/loader.rs', 'src/vault.rs', 'crates/notist-cli/src/ssg.rs']),
            ]:
                source = directory / mode
                shutil.copytree(template, source)
                rewrite(source)
                for file in files:
                    subprocess.run(['rustfmt', '--emit', 'stdout', '--edition', '2024',
                                    '--config', 'skip_children=true', str(source / file)],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
                evaluation = (source / 'crates/notist-pipeline/src/code/eval.rs').read_text()
                publication = (source / 'crates/notist-cli/src/publication.rs').read_text()
                self.assertEqual('eval.execute.runtime' in evaluation, mode == 'evaluator')
                self.assertEqual('ownership.directory_open' in publication, mode == 'io')
        self.assertEqual((REPO / 'crates/notist-pipeline/src/code/eval.rs').read_bytes(), original)
        if shutil.which('cc'):
            subprocess.run(['cc', '-fsyntax-only', '-Wall', '-Wextra',
                            str(REPO / 'tools/residual_opendir_probe.c')], check=True)
        events = allocator_events('[residual-allocator] pid=42 kind=malloc size=32816 malloc_calls=1 wall_ns=100000000 cpu_ns=99000000 path=/fixture/site/dist/bench\n')
        self.assertEqual(events[0]['size'], 32816)
        self.assertEqual(events[0]['cpu_s'], 0.099)
        self.assertEqual(events[0]['path'], '/fixture/site/dist/bench')


if __name__ == '__main__':
    unittest.main()
