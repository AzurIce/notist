"""Fixture and measurement guardrails; full integration is run by the driver."""
import json
from pathlib import Path
import tempfile
import unittest

from profile_module_site import FIELDS, MODES, fixture, modules, verify_cache_work


class ModuleSiteTests(unittest.TestCase):
    def test_fixture_has_complete_routes_and_resolvable_imports(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode", prefix="module-site-unit-") as name:
            root = Path(name)
            fixture(root, 65, "shared")
            spec = json.loads((root / "fixture.json").read_text())
            self.assertEqual(spec["page_count"], 68)
            self.assertEqual(len(list((root / "site/content").rglob("*.not"))), 68)
            import re
            for source in (root / "site").rglob("*"):
                if source.suffix not in [".not", ".notc"]: continue
                for target in re.findall(r'(?:#)?import "([^"]+)"', source.read_text()):
                    self.assertTrue((source.parent / target).is_file(), (source, target))

    def test_edit_sequence_preserves_name_spans_for_body_edits(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode", prefix="module-site-unit-") as name:
            root = Path(name)
            fixture(root, 8, "shared")
            steps = {s["name"]: s for s in json.loads((root / "fixture.json").read_text())["scenarios"]}
            original = modules()["version.notc"]
            changed = steps["body"]["overrides"]["modules/version.notc"]
            self.assertEqual(len(original.encode()), len(changed.encode()))
            self.assertEqual(changed, original.replace("stamp-A", "stamp-B"))
            self.assertTrue(steps["export"]["overrides"]["modules/version.notc"].startswith(changed))
            self.assertEqual(steps["missing"]["overrides"], steps["missing-warm"]["overrides"])
            self.assertEqual(steps["body"]["overrides"], steps["repaired"]["overrides"])

    def test_no_imports_is_a_real_negative_control(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode", prefix="module-site-unit-") as name:
            root = Path(name)
            fixture(root, 3, "none")
            for p in (root / "site/content").rglob("*.not"):
                self.assertNotIn("#import", p.read_text())
            self.assertFalse((root / "site/modules").exists())
            self.assertEqual(len(FIELDS), 13)

    def test_three_repetitions_balance_each_mode_position(self):
        orders = [MODES[i:] + MODES[:i] for i in range(3)]
        for position in range(3):
            self.assertCountEqual([order[position] for order in orders], MODES)

    def test_disabled_cache_cannot_silently_skip_work(self):
        sample = {"mode": "off", "workload": "private", "steps": [{"scenario": "cold", "stages": {
            "compile": {"calls": 2}, "compile_work": {"calls": 1}, "bind": {"calls": 2}, "bind_work": {"calls": 2}}}]}
        with self.assertRaises(AssertionError): verify_cache_work(sample)


if __name__ == "__main__":
    unittest.main()
