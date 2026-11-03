# Resource/Data historical regression attribution

This profiles the regression around `ef38d1b`, not just the new Data parser. All Rust source rewrites are restricted to result-directory copies. The main checkout's existing uncommitted SSG policy and matrix tooling remain untouched.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_resource_overhead.py prepare \
  --out /tmp/opencode/notist-resource-profile-NEW \
  --history /tmp/opencode/notist-history-matrix-20261103
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_resource_overhead.py build \
  --out /tmp/opencode/notist-resource-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_resource_overhead.py probes \
  --out /tmp/opencode/notist-resource-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_resource_overhead.py observed \
  --out /tmp/opencode/notist-resource-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_resource_overhead.py measure \
  --out /tmp/opencode/notist-resource-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_resource_overhead.py trace \
  --out /tmp/opencode/notist-resource-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_resource_overhead.py report \
  --out /tmp/opencode/notist-resource-profile-NEW
```

Default CPU 8, three fresh-process rounds, one complete warmup per measurement cell. Builds, probes, ordinary measurements and tracing are sequential, not concurrent. Prior timing samples are never reused.

**RSS caveat:** the retained wait4 field includes the child's fork-before-exec high-water mark inherited from the Python launcher. An independent trivial-child control confirms that increasing launcher allocation also increases this value. The report therefore makes no RSS ranking or cross-batch memory claim; use an exec-isolated collector before interpreting future low-memory cells as Rust-only peaks.

## Main measurements

The same complete five workloads and pinned inputs are copied from the historical matrix: complete XIV DB (51,739 HTML), literal/shared/private/wide sites (521 HTML each). Every measured child exits before any output checking starts. Every output is checked against the historical per-file SHA-256 manifest, including all pages, templates and assets.

There are 43 cells:

- 15 ordinary historical binary cells: environment/resource/current × five workloads.
- 20 diagnostic binary cells: resource/current × five workloads × full/no-scans.
- Eight further current diagnostic cells: literal/private × four narrower scan ablations.

Each diagnostic run has fine-grained resource counting/timing disabled; coarse stage probes and current-version module profiling remain identical across diagnostic modes. Ordinary binaries, disabled-resource-counter full-policy diagnostic binaries, timed probes, and unsafe scan ablations are recorded separately. Do not substitute probe wall time for ordinary CLI wall time.

## Fine-grained attribution

`tools/resource_profile_probe.rs` is a standalone counter/timer sink copied into notist-core **only in the temporary source snapshots**. Phases distinguish initial physical source acquisition, document/module analysis, evaluation, and remaining work.

- Time the full `ResourceSession.validate`, its file-revision loop and its kind loop separately.
- Count actual loop iterations, cache hits/misses, requests and filesystem revision/open calls. Record observed-set maxima.
- Time actual bounded reads, grants/session work, Data native calls, parse/decode and parse/projection cache calls.
- High-frequency filesystem operations are only counted in the main probes. Separate `RESOURCE_PROFILE=detail` runs time individual revision/open calls, making clock overhead visible instead of mixing it into the main ranking.
- The five existing workloads do not call language Data APIs. A genuine JSON/TOML positive control calls the example twice and must show four reads, two actual parses, two parse-cache hits, and actual decodes. This rules out disconnected probes as the explanation for zero counters.
- Timings are inclusive. File/kind loops are disjoint, but parent and child timings must never be summed together.

`observed` builds an additional CLI-wrapper probe by temporarily editing three files in the disposable current snapshot and restoring their exact contents in `finally`. The standalone sink is `tools/observed_reader_probe.rs`; the supplementary patch and binary hash are retained. Three separate detail runs distinguish `ObservedReader.revision`, path joining, locking/inserting into the observed set, inner `FsReader.revision`, and `site.data` loading. These are more intrusive and are not used for the main timings or subtracted from other runs.

## Unsafe diagnostic ablations

`RESOURCE_DIAGNOSTIC` is recognized only by these disposable diagnostic builds:

| Mode | Repeated full-batch checks omitted |
| --- | --- |
| full | None: original short-circuit behavior preserved |
| no-files | Previously observed file revisions |
| no-kinds | Previously observed resource kinds |
| no-source-scans | Full scans triggered by `source_bytes`, including byte-cache hits |
| no-eval-scans | Full scans immediately before/after a document evaluation |
| no-scans | All repeated full-batch scans |

Sticky changed-input failures, request accounting, data grants, capability-rooted reads and first-load before/after revisions remain. **Nevertheless, every non-full mode intentionally weakens the original batch-consistency semantics. Matching output on immutable benchmark inputs does not establish safety under edits. These are attribution controls, not optimizations or candidate production policies.**

## Independent syscall evidence

`strace -c` runs a separate 64-detail/66-HTML literal site, comparing the three ordinary binaries and full/no-scans current diagnostics. This smaller site is explicitly separate from the 521-page main timings; trace wall time is not a performance result. Source/output fingerprints and raw syscall summaries remain under `trace/`.

The evidence verifier also checks the work-count identities: initial source scans perform N×(N−1)/2 observed-file checks, evaluation performs twice the page count times the final observed-file count, and all such counts match between the resource commit and current HEAD. Tracing must show two additional statx and one close for every additional openat2, without increasing read calls. The selected syscall summary is not a count of all syscalls in the process.

## Provenance

Historical binary hashes, source fingerprints, input hashes, normal/current committed revision, toolchain, build settings, temporary patches, complete output hashes, stdout/stderr, timing samples and counters are retained. Resource versions receive only the identical temporary CLI request-budget override of 1,000,000 used in the matrix. No production defaults, consumer files, locks, page enumeration or `site.data` model is changed.

`report` emits a self-contained `dashboard.html` from `tools/resource_overhead_chart.html`, `chart-data.json`, complete `summary.json`/`summary.csv`, deterministic `integrity.json`, and the written report. The dashboard separates single-probe attribution from three-round disabled-counter diagnostic comparisons.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools -p 'test_*.py' -v
```
