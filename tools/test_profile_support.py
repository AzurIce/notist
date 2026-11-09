import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from profile_support import digest, inventory, save, target_directory


class ProfileSupportTests(unittest.TestCase):
    def test_inventory_preserves_wire_format_order_and_ignores_python_cache(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "nested").mkdir()
            (root / "__pycache__").mkdir()
            (root / "__pycache__/cache").write_bytes(b"ignored")
            for name, data in [("z", b"last"), ("nested/a", "中文".encode())]:
                (root / name).write_bytes(data)
            expected = {name: [len(data), hashlib.sha256(data).hexdigest()]
                        for name, data in [("nested/a", "中文".encode()), ("z", b"last")]}
            self.assertEqual(inventory(root), expected)
            self.assertEqual(list(inventory(root)), list(expected))
            self.assertEqual(digest(root / "z"), expected["z"][1])

    def test_json_evidence_format_remains_utf8_indented_and_newline_terminated(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "data.json"
            value = {"label": "中文", "numbers": [1, 2]}
            save(path, value)
            self.assertEqual(path.read_text(), json.dumps(value, indent=2, ensure_ascii=False) + "\n")

    def test_build_variants_are_isolated_without_an_opt_in_flag(self):
        root = Path("/evidence/target")
        paths = {target_directory(root, kind) for kind in ["before", "after", "before-ledger", "after-ledger", "time", "alloc"]}
        self.assertEqual(len(paths), 6)
        for label in ["", ".", "..", "../escape", "/absolute", "a/b", "a\\b"]:
            with self.assertRaises(ValueError):
                target_directory(root, label)
