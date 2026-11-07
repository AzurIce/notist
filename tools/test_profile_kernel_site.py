import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from profile_kernel_site import REPO, instrument


class KernelSiteProfileTests(unittest.TestCase):
    def test_disposable_probes_parse_and_keep_release_sources_untouched(self):
        relative = Path("crates/notist-ssg/src/build.rs")
        before = (REPO / relative).read_bytes()
        for detail in [False, True]:
            with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
                source = Path(directory)
                shutil.copytree(REPO / "src", source / "src")
                shutil.copytree(REPO / "crates", source / "crates")
                instrument(source, detail)
                body = (source / relative).read_text()
                self.assertIn('measure("pipeline.evaluate"', body)
                self.assertIn('"pages.item.evaluate"', body)
                self.assertIn('"pages.catalog.evaluate"', body)
                self.assertIn('measure("ssg.drop.values"', body)
                self.assertEqual('"pages.item.html"' in body, True)
                self.assertEqual('measure("body.html", || notist::xiv_profile::measure' in body, detail)
                cli = (source / "crates/notist-cli/src/ssg.rs").read_text()
                self.assertIn('measure("vault.drop", || drop(vault))', cli)
                self.assertIn('measure("artifact.drop", || drop(result))', cli)
                for path in [relative, Path("crates/notist-cli/src/ssg.rs")]:
                    subprocess.run(["rustfmt", "--emit", "stdout", "--edition", "2024",
                                    "--config", "skip_children=true", str(source / path)],
                                   stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True)
        self.assertEqual((REPO / relative).read_bytes(), before)


if __name__ == "__main__":
    unittest.main()
