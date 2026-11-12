# Resource identities and publication planning

## Results

Fresh-process, warm-filesystem full-site release medians from this A/B cohort:

| Workers | Clean rounds per revision | Elapsed before / after | CPU before / after | Peak RSS before / after |
| --- | ---: | ---: | ---: | ---: |
| 8 | 3 | 4.653 / 4.102 s | 7.744 / 7.322 s | 1.576 / 1.591 GiB |
| 16 | 3 | 4.573 / 3.998 s | 9.469 / 8.855 s | 1.583 / 1.601 GiB |
| 32 | 5 | 4.509 / 3.916 s | 12.587 / 10.938 s | 1.609 / 1.625 GiB |

Elapsed medians improve **11.83%, 12.58% and 13.14%** at 8, 16 and 32 workers;
combined CPU medians improve **5.45%, 6.49% and 13.10%**. All five 32-worker
candidate samples (3.903–3.993 s) are faster than every baseline sample
(4.491–4.599 s). These are small local cohorts, not cross-machine guarantees.

The tradeoff is **15–18 MiB higher peak RSS**, approximately **0.9–1.1%**, for
explicit identity/plan structures. Bounded byte comparison is not a claim of a
lower whole-build memory peak. The one candidate serial run is 6.528 s; it is a
compatibility/operating-point sample, not a paired serial performance result.

All **32 complete builds** reproduce all **78,004 output files**, including
path, byte length and SHA-256. Final source, binary and consumer-input
fingerprints are unchanged. The consumer check verifies 51,225 static item
pages, 513 catalog pages and every local HTML link. All three new perf
recordings have zero lost samples and no duration anomaly.

### Time attribution

Two-trace medians at 32 workers, measured separately from clean release samples:

| Root stage | Before | After |
| --- | ---: | ---: |
| Assets/Sass | 0.763 s | 0.461 s |
| Publication/actual unchanged-output comparison | 0.478 s | 0.246 s |
| Analysis/checking | 0.902 s | 0.857 s |
| Evaluate and prepare | 0.530 s | 0.547 s |
| Ordered reference/HTML finalization | 0.637 s | 0.631 s |
| Templates | 0.201 s | 0.177 s |

Assets and publication together decrease by about **0.533 s** in these traced
stage medians, consistent with the intended serial-work reduction. This is
not a subtraction of sampled CPU percentages, nor an additive decomposition
of the clean median: independently calculated phase medians need not sum to
the median overall duration. Environment/module preparation, nested inside
analysis, is 0.341 / 0.295 s; its implementation is not changed. Parsing is
0.120 / 0.120 s. No claim of a parser or evaluator speedup follows from this
change.

### Remaining CPU work

The three fresh candidate recordings show native path operations at
**10.01–11.39%** of user-mode cycles and component comparison at
**3.79–4.32%**. The separately measured prior-main cohort reported
15.92–18.26% and 9.41–10.41%, respectively. Those previous percentages are
reference context, not a newly paired CPU baseline or absolute seconds saved.

Allocator functions remain **19.18–21.28%** of candidate user cycles. The
representative recording is `after-perf-2`, chosen by proximity to the clean
elapsed median. Different denominators mean that a similar allocator fraction
does not imply unchanged absolute allocator work. Full raw recordings and
raw-period-weighted category data are retained in the evidence directory.

## Design boundaries

- `ResourceSession` interns each resolved native path once per batch. Bytes,
  kind, registered source and UTF-8 projection share a resource record. Rooted
  input caches use private record IDs; IDs confer no authorization.
- Every logical input read still consumes its request budget. Data reads check
  the current root before consulting that root's cache. First byte/error results,
  source overrides, parsed-data versions and bounded-allocation rules remain.
- `Assets` owns numbered destination records. A native-component index separates
  exact lookup from file/ancestor/descendant collision checks. Special paths use
  the standard `Path`-ordered compatibility index. No lossy UTF-8 key is used.
- External output maps retain standard `Path` order. Sorting sibling components
  avoids repeatedly comparing complete paths with long common prefixes.
- `ResolvedPath` lets CLI inspection and the filesystem operation share lexical
  resolution. It is not a capability or metadata cache. Symlink safety checks
  remain live on each operation; observed paths are sorted at the public boundary.
- `OutputPlan` borrows completed output bytes and exposes directory cursors and
  plan-local parent identities. It neither captures new input nor retains page
  trees. The native reader compares actual bytes using one reusable 64 KiB
  buffer across files and shares regular-file/root/metadata checks with reads.
- Third-party readers retain a bounded-read default. Any mismatch or error forces
  ordinary complete staging. Ownership checks, stale-output pruning, directory
  replacement and failed-replacement recovery remain. There is no timestamp or
  manifest-hash shortcut and no prefetch of the resource union.

The model and API contract are in `docs/designs/resource-publication.not`.
Environment preparation, parsing, Tera and evaluator internals are outside this
change. Output bytes remain owned by the completed artifact; the comparison
buffer is bounded, not the size of the entire publication artifact.

## Correctness validation

The final workspace run passes **1,032 Rust tests**, with no failures or ignored
tests. The new profiling driver passes its two Python tests. Formatting and
`git diff --check` pass.

Coverage includes existing source/data authorization and budget tests, shared
batch identity, request exhaustion, exact/prefix collision equivalence, native
non-UTF-8 identities, standard export order, reservation diagnostics, copy probe
ordering, directory identity, borrowed bytes, cross-buffer comparison and file
lengths, rooted access, symlinks, reader fallback and resolved-operation parity.

The first exploratory workspace run failed in
`preview_watches_concrete_hidden_data_inputs` with TCP `Connection reset by peer`.
The targeted five-test CLI SSG suite and both subsequent workspace runs passed.
The original failure log is retained. This does not establish that the failure
also occurs on the baseline; no unrelated networking/test retry change is made.

## Measurement contract

- Baseline runtime: `51878815c1876428cad283a3338cb0c6f0272242` on `main`.
- Candidate runtime commit: `8baf91a` on `main`; its archived tree matches the
  frozen source used for the release builds and all acceptance measurements.
- Candidate source starts from `git archive HEAD`, overlaying only the eight
  task-scoped runtime/test/design files listed in `profile_resource_plan.SCOPED`.
  The original sixteen modified/untracked files are excluded from the overlay
  and remain byte-identical to their saved fingerprints.
- Baseline release and frozen consumer input come from
  `/tmp/opencode/notist-main-5187881-profile-20261112`. The candidate normal and
  frame-pointer releases are fresh builds in separate Cargo targets, with the
  same compiler and release environment. Each build uses two compiler jobs.
- The full icexiv-db consumer runs in a fresh process with warm filesystem caches.
  Clean measurements include extension commands, resource work, body generation,
  templates, publication and actual unchanged-output comparison. No interposer,
  timing export or perf instrumentation is active in the clean measurements.
- One before/after warmup; five alternating clean rounds at 32 workers and three
  at each of 8 and 16 workers; one candidate serial run; two timing captures per
  revision; three candidate `cycles:u` recordings at 32 workers, 499 Hz, with
  frame-pointer stacks. This is **32 complete builds**. Every invocation verifies
  all 78,004 output paths, byte lengths and SHA-256 outside its measured interval.
- Clean wall/CPU/RSS results are paired within this experiment. CPU means waited
  user+system CPU including children; RSS is the reported process high-water
  mark, not a sum across simultaneously live processes. Warmup, timing and perf
  runs do not enter clean medians.
- Nested trace durations overlap their enclosing stage; worker-inclusive sums
  are not elapsed time. Tracing export/shutdown is outside captured trace time
  but inside the instrumented process duration.
- CPU operation categories are mutually exclusive physical leaf functions,
  weighted by raw periods. They describe user-mode cycles, not kernel work,
  waiting or elapsed-time shares. Prior-cohort CPU percentages, if referenced,
  are explicitly separate measurements, not a newly paired perf baseline.
- All recordings are retained. A recording exceeding twice the candidate clean
  median is marked anomalous; a representative typical recording is selected
  by its proximity to that clean median, not by preferred hotspot percentages.

## Evidence and reproduction

- Full-site evidence: `/tmp/opencode/notist-resource-plan-profile-20261112`.
- Workspace logs and original-work backup:
  `/tmp/opencode/notist-resource-plan-20261112`.
- Driver: `tools/profile_resource_plan.py`.
- Driver tests: `tools/test_profile_resource_plan.py`.
- Analysis: `resource-plan-analysis.json`; its `reproducer/analyze_evidence.py`
  uses a byte-for-byte archived copy of the existing `analyze_merged_main.py`
  classifier. The original classifier and prior reports are not edited.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_resource_plan.py all \
  --out /tmp/opencode/notist-resource-plan-reproduction
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools \
  -p test_profile_resource_plan.py
```

Use a fresh evidence directory. `--reference` selects an archived copy of the
baseline release, frozen consumer snapshot and expected output inventory.
The evidence's archived analysis can be rerun with:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 \
  /tmp/opencode/notist-resource-plan-profile-20261112/reproducer/analyze_evidence.py \
  /tmp/opencode/notist-resource-plan-profile-20261112
```
