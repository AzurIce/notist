"""Small guardrails for the history benchmark's comparison/probe machinery."""
import tempfile
from pathlib import Path
import unittest

from profile_xiv_history import CASES, ENGINES
from xiv_history_probes import SemanticHTML, enter_function, inventory, measure_rhs


class HistoryTests(unittest.TestCase):
    def test_error_propagation_stays_outside_probe_closure(self):
        source = "let analysis = vault.analyze(&route, &text)?;\n        errors.extend([]);"
        self.assertEqual(
            measure_rhs(source, "let analysis = ", ";\n        errors.extend", "analyze"),
            'let analysis = notist::xiv_profile::measure("analyze", || vault.analyze(&route, &text))?;\n        errors.extend([]);',
        )

    def test_probe_can_wrap_infallible_expression(self):
        self.assertIn('|| render());', measure_rhs("let html = render();\nend", "let html = ", ";\nend", "render"))

    def test_function_entry_only_targets_exact_name(self):
        source = "fn built() {}\nfn build<R: Resource>(x: R) -> Result<(), Error> {\n    Ok(())\n}"
        result = enter_function(source, "build", "    let probe = (); ")
        self.assertTrue(result.startswith("fn built() {}"))
        self.assertIn("{\n    let probe = ();", result)
        with self.assertRaises(ValueError):
            enter_function(source, "missing", "")

    def test_semantics_normalizes_only_indentation_attributes_and_entities(self):
        def tokens(text):
            parser = SemanticHTML()
            parser.feed(text)
            parser.close()
            return parser.tokens
        self.assertEqual(tokens('<p id="x" class="y">a &amp; b</p>\n'),
                         tokens('\n<p class="y" id="x">a &#38; b</p>'))
        self.assertNotEqual(tokens("<p>a b</p>"), tokens("<p>ab</p>"))
        self.assertNotEqual(tokens("<p>a</p>"), tokens("<p>a</p><!--x-->"))

    def test_inventory_checks_paths_and_bytes(self):
        with tempfile.TemporaryDirectory(prefix="xiv-history-unit-", dir="/tmp/opencode") as name:
            root = Path(name)
            (root / "a.html").write_text("<p>ok</p>")
            first = inventory(root, semantic=True)
            (root / "a.html").rename(root / "b.html")
            second = inventory(root, semantic=True)
            self.assertNotEqual(first["sha256"], second["sha256"])
            self.assertEqual(first["files"], 1)
            self.assertEqual(first["html"], 1)

    def test_each_round_contains_all_cases(self):
        for i in range(5):
            shift = (i * 5) % len(CASES)
            order = CASES[shift:] + CASES[:shift]
            if i % 2:
                order = list(reversed(order))
            self.assertCountEqual(order, CASES)
        self.assertEqual(len({n for n, *_ in ENGINES}), 15)
        self.assertEqual(len({n for n, *_ in CASES}), 18)
        self.assertEqual(next(rev for name, rev, _ in ENGINES if name == "head"), "2468767")


if __name__ == "__main__":
    unittest.main()
