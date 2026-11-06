import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import profile_runtime_cow as profile


class RuntimeCowProfileTests(unittest.TestCase):
    def test_prepare_overlays_only_runtime_files_not_concurrent_edits(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            root = Path(directory)
            out, repo, cohort = (root / name for name in ["out", "repo", "cohort"])
            before = out / "sources/before"
            before.mkdir(parents=True)
            for relative in profile.FILES:
                path = repo / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("after\n")
                if not relative.endswith("instance_cache.rs"):
                    target = before / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text("before\n")
            (before / "Cargo.lock").write_text("original lock\n")
            (repo / "Cargo.lock").write_text("unrelated concurrent edit\n")
            (cohort / "site").mkdir(parents=True)
            (cohort / "site/input.not").write_text("frozen input\n")
            profile.save(cohort / "manifest.json", {"inputs": profile.inventory(cohort / "site")})
            profile.save(cohort / "output-inventory.json", {})
            profile.save(out / "manifest.json", {"before_sources": profile.inventory(before), "binaries": {}})
            args = SimpleNamespace(out=out, repo=repo, cohort=cohort, rounds=6)
            with patch.object(profile.subprocess, "check_output", return_value="test rustc"), contextlib.redirect_stdout(io.StringIO()):
                profile.prepare(args)
            self.assertEqual((out / "sources/after/Cargo.lock").read_text(), "original lock\n")
            manifest = profile.guard(args)
            self.assertEqual(manifest["runtime_files"], profile.FILES)
            (out / "sources/after/Cargo.lock").write_text("unexpected change\n")
            with self.assertRaises(AssertionError):
                profile.guard(args)

    def test_report_excludes_warmup_and_pairs_each_round(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            out = Path(directory)
            rows = []
            for rep, before, after in [(-1, 1000, 1), (0, 10, 9), (1, 20, 18)]:
                for kind, wall in [("before", before), ("after", after)]:
                    rows.append({"kind": kind, "round": rep, "wall_s": wall, "user_s": wall, "system_s": 0})
            profile.save(out / "samples.json", rows)
            with patch.object(profile, "guard", return_value={}), contextlib.redirect_stdout(io.StringIO()):
                profile.report(SimpleNamespace(out=out))
            result = json.loads((out / "summary.json").read_text())
            self.assertEqual(result["before"]["wall_s"], 15)
            self.assertEqual(result["after"]["wall_s"], 13.5)
            self.assertEqual(result["paired_gain_percent"], [10, 10])
            self.assertEqual(result["faster_rounds"], 2)

    def test_build_resumes_tests_and_cleans_between_feature_modes(self):
        with tempfile.TemporaryDirectory(dir="/tmp/opencode") as directory:
            out = Path(directory)
            for relative in ["logs", "bin", "sources/before", "sources/after"]:
                (out / relative).mkdir(parents=True)
            binaries = {}
            for kind in profile.KINDS:
                (out / "bin" / kind).write_text(kind)
                binaries[kind] = profile.digest(out / "bin" / kind)
            manifest = {"binaries": binaries, "test_modes": {"before": ["default", "all"]}}
            metadata = json.dumps({"packages": [], "workspace_members": []})
            with patch.object(profile, "guard", return_value=manifest), \
                 patch.object(profile.subprocess, "check_output", return_value=metadata), \
                 patch.object(profile.subprocess, "run") as run, \
                 contextlib.redirect_stdout(io.StringIO()):
                profile.build(SimpleNamespace(out=out))
            commands = [call.args[0] for call in run.call_args_list]
            self.assertEqual(len(commands), 4)
            self.assertEqual(commands[0][:3], ["cargo", "clean", "--release"])
            self.assertNotIn("--all-features", commands[1])
            self.assertEqual(commands[2][:3], ["cargo", "clean", "--release"])
            self.assertIn("--all-features", commands[3])
            self.assertEqual(manifest["test_modes"]["after"], ["default", "all"])


if __name__ == "__main__":
    unittest.main()
