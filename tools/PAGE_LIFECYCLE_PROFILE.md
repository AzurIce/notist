# Bounded page lifecycle

## Scope

The worktree is `bounded-page-lifecycle`, branch
`refactor/bounded-page-lifecycle`, based on `64f0092`. The baseline is that exact
main release, not the pre-allocation-optimization revision.

## Results

The verified cohort shows lower elapsed time, total CPU and peak RSS, alongside
the tested bound on evaluated-tree residency. These are results for this frozen
workload and machine, not a universal speedup guarantee. Earlier noisy cohorts
are retained separately and do not enter these medians.

Final clean cohort, medians including every sample:

| Workers / rounds | Metric | Main 64f0092 | Bounded lifecycle | Change |
| --- | --- | ---: | ---: | ---: |
| 32 / 5 pairs | Combined user+system CPU | 14.876 s | 11.809 s | -20.61% |
| 32 / 5 pairs | Peak RSS | 2.215 GiB | 1.622 GiB | -26.79% |
| 32 / 5 pairs | Elapsed time | 5.560 s | 4.416 s | -20.57% |
| 16 / 3 pairs | Combined user+system CPU | 10.766 s | 9.463 s | -12.11% |
| 16 / 3 pairs | Peak RSS | 2.211 GiB | 1.598 GiB | -27.71% |
| 16 / 3 pairs | Elapsed time | 5.554 s | 4.484 s | -19.27% |

At 32 workers, verified main samples span 5.512–5.681 s and the candidate samples
4.374–4.726 s; all five candidate measurements are faster. Separate final timing
captures complete at 6.071/6.000 s for main and 4.971/4.960 s for the candidate.
The diagnostic runs do not replace clean medians. An earlier intermediate
cohort had much larger elapsed-time variation despite stable CPU/RSS; its wait
source is not established, and its samples are neither discarded from that
cohort nor combined with this delivery cohort.

All **23 final full-site invocations** produced the same **78,004 files**.
The consumer check verifies 51,225 item pages, 513 catalog pages and every local
HTML link. The serial candidate run also matches all output bytes.

### Phase interpretation

Separate 32-worker diagnostic captures, two runs per binary; inclusive phase
durations are not clean wall-time or CPU measurements:

| Phase | Main s | Bounded lifecycle s |
| --- | ---: | ---: |
| Analysis | 0.920 | 0.927 |
| Evaluate/prepare plus ordered render combined | 2.235 | 1.181 |
| Evaluate/prepare, within the combined row | 0.786 | 0.550 |
| Ordered render, within the combined row | 1.449 | 0.631 |
| Links, nested in ordered render | 0.685 | 0.438 |
| Assets | 0.739 | 0.771 |
| Templates | 0.283 | 0.203 |
| Publication | 0.470 | 0.464 |

Combining evaluate/prepare with render avoids claiming a speedup merely because
HTML generation and destruction moved out of the serial render span. Those
operations now run on evaluation workers. The sum of HTML-plan preparation
spans is 1.007 s and finalization spans 0.109 s, versus immediate HTML render
spans of 0.347 s on main. Preparation performs additional projection/encoding;
parallel inclusive sums cannot be added to stage wall time or interpreted as
process CPU. Asset preparation is slower in these captures, rather than being
silently excluded from the result.

The whole-process CPU reduction remains independently measured. There is no
new allocator-call census or cycles profile in this cohort, so the previous
~40% allocator share is not reused as either a current share or saved time.
Raw samples, timing captures and `stage-summary.json` are in the final evidence.

## Ownership boundary

The implementation keeps the whole-site analysis/source-registration barrier.
Workers evaluate a document, prepare its metadata/reference facts and HTML
plan, then release its evaluated IR before starting their next document. Only
tree-free page projections are collected. Input analyses are retired in windows
of 128 documents per worker. Resources are not prefetched during preparation.

After evaluation, the host prepares ordinary assets and the complete route/ID
index, then finalizes pages in order: capture referenced assets, resolve links
and anchors, select component entries, materialize HTML, publish component
resources. The complete Pages/HTML remain available to site extensions/templates.

This bounds evaluated tree residency, not total build memory. Whole-site analysis,
source/module snapshots, parsed inputs, HTML plans, template data and final files
can still scale with site size. There is no allocator replacement, second
evaluation pass, disk spill of IR, or omission of publication work.

Design: `docs/designs/page-lifecycle.not`.

## Semantic checks

Final workspace run: **1020 passed, 0 failed, 0 ignored** across 113 suites.
The profile tool's three unit tests also pass. Workspace log:
`/tmp/opencode/notist-bounded-page-lifecycle-verified-workspace-tests.log`.

- A weak-reference test injects uniquely owned strings in `after_evaluate`, so
  analysis and caches cannot own them. Across 521 pages at 1/4 workers, no more
  than the worker budget's number of evaluated trees are live; all injected IR
  strings are gone while returned Pages/HTML still exist.
- Cross-window forward anchors and typed fragments have identical serial/parallel
  outputs. Generated/edited document reads continue using registered source text.
- A mutable-resource fixture creates a component implementation and changes an
  image only in the final output hook of 257 pages. Component probes and
  publication reads must occur afterward, including across preparation windows.
- HTML plans are compared against immediate rendering for safety, suffix conflicts,
  source diagnostics, nested components with/without implementations, and native
  structural layout. Independent reference occurrences use local numeric keys,
  not `(origin, target)` as their presentation identity.
- Binding plans retain the document's batch/configuration identity; a different
  Vault cannot resolve them. Plans retain no evaluated Item or Sources aggregate.
- Repeated typed references reuse their page-local resource base even when one
  occurrence fails anchor validation, preserving the single anchor error without
  adding an unpublished-resource error. Both reference orders are tested.
- Existing Book/mount/extension/serve/publication tests cover recovery, source
  locations, collisions and preserving the last successful output on failure.

HTML timing is split into `core.render.html.prepare` (on the evaluation worker)
and `core.render.html.finish` (ordered finalization). The whole build is still
timed; a smaller serial render span alone is not a claimed speedup.

## Measurement contract

- Final evidence: `/tmp/opencode/notist-bounded-page-lifecycle-verified-20261112`.
- Input/baseline origin:
  `/tmp/opencode/notist-main-64f0092-profile-20261112`.
- A fresh copy of tracked and nonignored worktree files, including new files,
  is inventoried and compiled in a separate release target. The baseline binary,
  compiler identity and build settings are checked against the reference manifest.
- Warmup both binaries; five interleaved pairs at 32 workers, three at 16.
  Every sample is retained, with no outlier exclusion. A serial candidate build
  independently checks output compatibility. Two separate timing runs per binary
  are diagnostic only, not mixed into clean timing medians.
- Normal full builds include the Python extension, resource capture, templates,
  publication and unchanged-output byte comparison. File inventory/hashing is
  outside the measured interval. Filesystem caches are warm; each invocation
  starts a fresh process. No heap interposer or timing instrumentation is enabled
  in normal measurements.
- CPU is wait4 user+system including waited-for children. RSS is the largest
  reported process high-water mark, not a simultaneous sum across processes.
- Every invocation verifies all 78,004 output paths, byte sizes and SHA-256.
  Final verification also checks frozen source/input/binary fingerprints and
  runs the consumer's `check_site.py`.

An exploratory cohort is retained separately at
`/tmp/opencode/notist-bounded-page-lifecycle-20261112`. It predates the final
batch-identity checks and diagnostic-location handling. Its samples are not
combined with the final cohort.

`/tmp/opencode/notist-bounded-page-lifecycle-final-20261112` is also retained as
an intermediate cohort preceding the repeated-reference diagnostic regression
fix. Its samples are not combined with the verified delivery cohort.

## Reproduce

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_page_lifecycle.py all \
  --out /tmp/opencode/notist-page-lifecycle-validation
cargo test --workspace --locked --offline -j2 -- --test-threads=4
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools \
  -p test_profile_page_lifecycle.py
```

The evidence directory must be fresh. `--reference` can select an archived copy
of the main-profile evidence. Normal release builds use the same compiler and
settings as the baseline and limit compilation to two jobs.

Strict Clippy validation is blocked by existing warnings/errors in unchanged
syntax/core code (including `mut_from_ref`, `box_collection` and
`large_enum_variant`); unrelated code is not rewritten or suppressed here.
