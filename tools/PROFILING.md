# Evaluator measurements

The maintained evaluator benchmark is `run_eval_bench.py` with the compiler-backed
`eval-bench` driver. It freezes sources, separates uninstrumented release timing
from allocation diagnostics, and checks every scenario's complete result outside
the measured region. See `eval-bench/README.md` for commands and limitations.

The `profile_*` scripts also contain historical, source-insertion experiments.
They are not a generic profiler API. Their anchors, overlay whitelists, previous
manifests and cohorts refer to particular frozen source layouts. In particular:

- The SSG optimization/residual probes are tested against `18f893a`.
- Boundary/checkpoint/source-text probes are tested against `e5c65a3`.
- Checkpoint field attribution explicitly rejects the closure kernel before
  changing a snapshot. That kernel has no call backups or execution-state fields;
  relabeling other allocations as those old operations would be misleading.

Historical probe tests use their pinned revisions rather than silently overlaying
the current working tree onto old experiments. Historical evidence and reports
must not be rewritten to describe the new kernel. New before/after comparisons
need fresh source manifests and version-appropriate semantic checks.
