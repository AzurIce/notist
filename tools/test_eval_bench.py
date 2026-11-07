"""Keep timing and diagnostic evidence distinct; never launch performance work."""
import copy
import unittest

from run_eval_bench import summarize


def evidence(mode, ns=1000):
    row = {
        "id": "native_text/1", "kind": "native_text", "size": 1,
        "facts": {"output_nodes": 3}, "module_source": "", "document_source": '#text(text: "x")',
        "timing": {"median_ns_per_eval": ns},
        "samples": [{"allocations": 8, "allocation_bytes": 512, "reallocations": 1,
                     "reallocation_requested_bytes": 64, "frees": 8,
                     "peak_extra_requested_bytes": 400, "output_live_extra_bytes": 300,
                     "net_live_after_drop_bytes": 0}],
    }
    return {"mode": mode, "allocation_instrumented": mode == "alloc",
            "debug_assertions": False, "rows": [row]}


class EvalBenchTests(unittest.TestCase):
    def test_reports_process_medians_without_filtering_slow_samples(self):
        result = summarize([evidence("time", ns) for ns in [1000, 9000, 3000]], evidence("alloc"))
        self.assertEqual(result[0]["median_ns"], 3000)
        self.assertEqual(result[0]["process_medians_ns"], [1000, 9000, 3000])
        self.assertEqual(result[0]["max_process_median_ns"], 9000)
        self.assertEqual(result[0]["allocation_medians"]["allocation_bytes"], 512)

    def test_instrumented_or_debug_time_is_rejected(self):
        for field in ["allocation_instrumented", "debug_assertions"]:
            run = evidence("time")
            run[field] = True
            with self.assertRaises(ValueError):
                summarize([run], evidence("alloc"))
        with self.assertRaises(ValueError):
            summarize([evidence("time")], evidence("time"))

    def test_changed_scenario_or_semantics_cannot_be_combined(self):
        for field in ["facts", "module_source", "document_source"]:
            diagnostic = evidence("alloc")
            diagnostic["rows"][0][field] = "changed"
            with self.assertRaises(ValueError):
                summarize([evidence("time")], diagnostic)
        changed = copy.deepcopy(evidence("time"))
        changed["rows"][0]["id"] = "different"
        with self.assertRaises(ValueError):
            summarize([evidence("time"), changed], evidence("alloc"))


if __name__ == "__main__":
    unittest.main()
