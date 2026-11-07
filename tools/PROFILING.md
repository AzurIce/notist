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

## Current closure-kernel whole-site profile

`profile_kernel_site.py` freezes the current source and dirty consumer inputs in
a fresh external directory. Its insertion anchors target the `c722ecb` kernel;
these scripts are reproducible experiments, not a stable instrumentation API.
They do not modify the consumer or production sources. Every successful measured
build checks all output paths, lengths and SHA-256 hashes outside the timer.

```sh
OUT=/tmp/opencode/notist-icexiv-kernel-fresh
python3 tools/profile_kernel_site.py prepare --out "$OUT" --consumer /path/to/icexiv-db
python3 tools/profile_kernel_site.py build --out "$OUT"
python3 tools/profile_kernel_site.py measure --out "$OUT"
python3 tools/analyze_kernel_site.py "$OUT"
python3 tools/profile_kernel_body.py --out "$OUT"
python3 tools/compare_kernel_site.py --out "$OUT" --baseline e07b67e
python3 tools/profile_kernel_baseline.py --out "$OUT"
python3 tools/profile_kernel_alloc.py --out "$OUT"
python3 tools/profile_kernel_cpu.py --out "$OUT" --profiler /path/to/libprofiler.so --pprof /path/to/pprof
```

Run sequentially, without another build using this cohort's disposable target.
Preparation requires the current game-export consumer layout; it is not a generic
site fixture. Build dependencies must already be available for Cargo offline mode.
Run the consumer's `scripts/check_site.py` in the frozen `site` directory afterward.

- Clean interleaved A/B (`paired-samples.json`) supplies latency/RSS evidence.
- Coarse probes supply exclusive top-level wall attribution (`summary.json`).
- Detail probes count recursive operations; inclusive times **cannot be added**.
- Body probes measure links, HTML and tree destruction at natural drop positions.
- CPU sampling uses libprofiler/libunwind, not tcmalloc; executable identity excludes
  Python children. It requires Linux/glibc and does not change perf permissions.
- Allocation diagnostics count malloc/calloc/realloc/free API traffic, not live
  memory. Reallocations also count as allocation events; requested bytes include
  realloc sizes, exclude aligned entrypoints and libc-internal calls, and are not RSS.
- Never substitute instrumented run latency for clean release latency. Filesystem
  caches are uncontrolled; a fresh CLI process does not imply cold disk caches.

Findings: `docs/_logs/2026-11-07-icexiv-kernel-profile.md`.

## SourceId32 isolated experiment

`profile_source_id.py` compares the frozen kernel profile's `normal` sources
against the current dirty workspace on exactly the same full frozen site. It
builds both releases, separates initial publication, interleaves five clean
pairs, verifies every output, and runs allocation diagnostics separately:

```sh
OUT=/tmp/opencode/notist-source-id32-fresh
python3 tools/profile_source_id.py prepare --out "$OUT" --reference /path/to/frozen-kernel-cohort
python3 tools/profile_source_id.py build --out "$OUT"
python3 tools/profile_source_id.py measure --out "$OUT"
python3 tools/profile_source_id.py allocations --out "$OUT"
```

Use a fresh external directory and run actions sequentially. The reference must
contain the complete, guarded `c722ecb` kernel cohort and its output inventory.
The candidate includes source-name interning and source-map COW; it does not
isolate 32-bit versus 64-bit handle width. The initial implementation retains
source names for the process lifetime, not source bodies, and is not a completed
reclaiming source database. `source_id_layout.rs` can be compiled against each
frozen core rlib to inspect actual struct layouts outside measured regions.

Findings: `docs/_logs/2026-11-07-source-id32.md`.
