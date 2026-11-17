# Checked static-resource capture

## Result

The portable capture/discovery change reduces the icexiv-db resource stage by
about 43% and normal full-build elapsed time by about 6% in this local cohort.
It does not materially reduce whole-build CPU or peak memory at 32 workers.

| Measurement | Before | After | Change |
| --- | ---: | ---: | ---: |
| Resource-stage trace median, 3 per revision | 0.690922 s | 0.393649 s | -43.03% |
| Clean full-build median, 32 workers, 5 per revision | 4.347154 s | 4.094653 s | -5.81% |
| Combined CPU median, 32 workers | 12.142128 s | 12.061168 s | -0.67% |
| Peak RSS median, 32 workers | 1.6218 GiB | 1.6236 GiB | +0.11% |
| Clean full-build median, 8 workers, 3 per revision | 4.568174 s | 4.312052 s | -5.61% |
| Combined CPU median, 8 workers | 7.843759 s | 7.549068 s | -3.76% |

All five clean 32-worker candidate runs (4.077–4.113 s) are faster than every
baseline run (4.306–4.359 s). The 8-worker candidate cohort retains a 5.009 s
sample alongside 4.214 and 4.312 s; nothing is discarded. These are small local
cohorts on a non-exclusive host, not cross-platform performance guarantees.

Trace samples are separate from clean samples. Before asset-stage durations are
0.766/0.691/0.664 s and after durations are 0.409/0.391/0.394 s. Changes in other
trace stages are not attributed to this resource-only implementation. The trace
medians do not form an additive decomposition of clean elapsed time.

## Implementation boundary

- `Resources::entries_with_kind` returns paths with optional classification
  hints. Its default implementation preserves lazy `entries`/`kind` fallback;
  hidden entries can be skipped before fallible classification. Native discovery
  reuses directory-entry file types and sorts the final file list once.
- `Resources::capture_file` expresses a complete regular-file read. The default
  is `kind` followed by `read`, preserving legacy resource backends. Reference
  adapters forward both new operations rather than losing native overrides.
- `FsResources::capture_file_resolved` opens a file, checks the opened object's
  metadata and reads owned bytes. Unix uses nonblocking/non-controlling-terminal
  open flags so a substituted FIFO cannot block before the regular-file check.
- CLI `Files` performs its existing complete lexical-path symlink inspection
  once per selected capture, not separately for copy classification and read.
  Typed enumeration still checks each traversed directory; hints are never
  capabilities or a substitute for capture-time checks.
- `Assets::copy_file` captures a known file without rediscovering its type. It
  retains source/output overlap, source deduplication and destination-collision
  checks. Shared insertion avoids a repeated target lookup to record provenance.
  Generic recursive `Assets::copy` remains available and keeps its probe order.
- Static/theme and selected content-asset copies use the file operation. Sass
  compilation, explicit-reference handling, extension protocols and publication
  are not rewritten. This icexiv-db input contains no Sass.

The content remains owned bytes. There is no lazy source path in the final
artifact, no new allocator, no extra worker pool, no Linux-only openat2 path and
no metadata/timestamp cache. Full-build output comparison still reads actual
bytes and changed output still uses staged publication.

## Semantics and limits

1. A discovered file replaced by a directory is rejected, rather than silently
   expanding that directory into output. `copy_file` intentionally requires a file.
2. Native typed enumeration conservatively records returned paths as observed
   dependencies, including hidden or subsequently shadowed entries. This can
   broaden preview invalidation; it does not authorize reads of those entries.
3. The CLI still checks the current logical path for links on every capture.
   It does not pin a directory across requests or cache a prior safety decision.
   The existing path-check/open race is not claimed to be eliminated; checking
   the opened object's regular-file type is not equivalent to atomic no-links
   path resolution. Directory anchoring requires a separate contract/design.
4. `FsResources` itself retains its ordinary link-following behavior. The CLI
   adapter owns the stricter link policy. Capture does not introduce a new
   rooted data grant or bypass ResourceSession logical read accounting.
5. The operation returns owned bytes but does not create a global capture cache
   or an atomic filesystem snapshot. Concurrent mutation of source files remains
   outside any claim of an all-files-at-once snapshot.
6. Runtime tests and benchmarks here ran on Linux. The implementation uses
   portable standard-library operations with Unix-only open flags under cfg.
   The portability workflow now includes Linux, Windows and macOS and runs the
   new resource-capture and typed-discovery tests; remote CI was not launched.

## Validation

- Final full workspace: **1,052 tests passed**, zero failed/ignored, with
  `cargo test --workspace --locked --offline -j2 -- --test-threads=4`.
- New tests cover legacy fallback and reference forwarding, typed discovery,
  hidden entries, native non-UTF-8 paths, owned bytes, missing/directory inputs,
  nonblocking FIFO rejection, symlink replacement after enumeration, source
  deduplication, source/output overlap and collision-before/after-read behavior.
- An initial workspace attempt failed in the preview integration test
  `preview_watches_external_extension_inputs_recursively_even_after_failure`
  with `ConnectionRefused`. Both subsequent full workspace runs passed. Logs are
  retained; there is no claim that the failure was established on baseline, and
  no unrelated networking/retry implementation was changed.
- **25 complete site builds**: 2 warmups, 10 normal 32-worker samples, 6 normal
  8-worker samples, 6 trace captures and 1 deliberate output-repair check.
- Every build reproduces all **78,004** output paths, byte lengths and SHA-256
  outside its measured interval. A tampered PNG forces real staged replacement
  and is restored correctly. This repair sample is not a clean timing sample.
- Consumer check verifies **51,225 item pages**, **513 catalog pages**, and all
  local HTML links. Final input, source and binary fingerprints are unchanged.
- Only changed Rust files were formatted; unrelated repository formatting is
  left unchanged. `git diff --check` passes.

## Measurement provenance

- Baseline: `45f3370acd3ada5f8a0707d905c5e1f2b38b9be6`.
- Candidate: that commit plus the frozen worktree overlay recorded in manifest.
- Worktree: `perf/ssg-resource-capture`.
- Evidence: `/tmp/opencode/notist-resource-capture-ab-20261117`.
- Input/baseline origin: `/tmp/opencode/notist-icexiv-current-20261117`.
- Host: AMD Ryzen 9 9950X3D2, 32 logical CPUs; rustc 1.97.1.
- Candidate is independently compiled release with debug level 1, incremental
  disabled and two compiler jobs, matching baseline settings. Existing normal
  baseline executable is reused after fingerprint verification; no old timing
  sample is reused. Both binaries are measured anew on one copied input snapshot.
- Full builds use fresh processes and warm filesystem caches. Includes Python
  extensions, resources, templates, publication and actual unchanged-output byte
  comparison; excludes Rust compilation, game export/prepare and extra hashing.
- Per setting, before/after order alternates. Tracing/export is never active in
  clean samples. CPU is waited-for user+system including children; RSS is the
  reported process high-water mark, not a simultaneous parent/child sum.
- `measure.py`, source, binaries, inventories, all samples, traces and test logs
  are retained in evidence. It is a one-shot frozen experiment, not a general
  profiler; reproduction requires a fresh evidence/site directory.

The remaining approximately 0.39 s is not evidence that the earlier 0.2–0.3 s
target has been reached. Full-path inspection is still performed once per file;
directory-relative capture or bounded parallel reads remain separate experiments.
