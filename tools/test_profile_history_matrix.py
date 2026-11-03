"""Guard the compatibility fixtures and historical comparison matrix."""
import json
from pathlib import Path
import re
import tempfile
import unittest

from profile_history_matrix import MATRIX_CASES, WORKLOADS, chart_payload, item_source, synthetic
from profile_module_site import record


class MatrixTests(unittest.TestCase):
    def test_full_matrix_keeps_the_historical_checkpoints(self):
        self.assertEqual(len(MATRIX_CASES), 18)
        self.assertEqual(len(WORKLOADS), 5)
        self.assertEqual(len({c[0] for c in MATRIX_CASES}), 18)
        self.assertEqual(MATRIX_CASES[-1][0:2], ("current", "current"))
        self.assertEqual({w[2] for w in WORKLOADS[1:]}, {521})

    def test_old_language_arguments_are_formatted_at_the_input_boundary(self):
        source = item_source(record(0), 0, "shared")
        self.assertIn('view.show("10001"', source)
        self.assertIn('true', source)
        self.assertNotIn('str(', source)
        wide = item_source(record(0), 0, "wide", wide=True)
        self.assertIn('"是"', wide)

    def test_fixtures_have_no_post_history_language_requirements(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode", prefix="history-matrix-unit-") as directory:
            for reuse in ["literal", "shared", "private", "wide"]:
                root = Path(directory) / reuse
                synthetic(root, reuse, pages=8)
                spec = json.loads((root / "fixture.json").read_text())
                self.assertEqual(spec["page_count"], 10)
                self.assertEqual(spec["scenarios"], [])
                for p in (root / "site").rglob("*"):
                    if p.suffix not in [".not", ".notc"]:
                        continue
                    text = p.read_text()
                    self.assertNotIn("str(", text)
                    self.assertNotIn("#for ", text)
                    self.assertNotIn(".len()", text)
                    for target in re.findall(r'(?:#)?import "([^"]+)"', text):
                        self.assertTrue((p.parent / target).is_file(), (p, target))
                tera = (root / "templates-tera/page.tera").read_text()
                handlebars = (root / "templates-handlebars/page.hbs").read_text()
                self.assertEqual(handlebars.replace("{{{page.html}}}", "{{ page.html | safe }}"), tera)

    def test_wide_export_definitions_all_have_real_call_sites(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode", prefix="history-matrix-unit-") as directory:
            root = Path(directory)
            synthetic(root, "wide", pages=8)
            libraries = "".join(p.read_text() for p in (root / "site/modules").glob("labels*.notc"))
            view = (root / "site/modules/view.notc").read_text()
            definitions = set(re.findall(r"fn (label_\d+)\(", libraries))
            calls = set(re.findall(r"(label_\d+)\(\)", view))
            self.assertEqual(len(definitions), 256)
            self.assertEqual(definitions, calls)
            self.assertEqual(set(re.findall(r"fn show(\d+)\(", view)), set(map(str, range(8))))

    def test_chart_preserves_units_ranges_and_missing_probe_counts(self):
        cell = {"wall_s": {"median": 2.0, "min": 1.9, "max": 2.1}, "rss_kib": {"median": 2048, "min": 1024, "max": 3072}}
        data = chart_payload({"workloads": [("sample", "样例", 1)], "cases": [{"case": "tera", "revision": "abcdef123", "workloads": {"sample": cell}}]})
        self.assertEqual(data["cases"][0]["values"]["sample"], [2.0, 1.9, 2.1, 2.0, 1.0, 3.0, None, None])


if __name__ == "__main__":
    unittest.main()
