# Multi-workload historical profiles

Each historical checkpoint has **five adjacent bars**, one per workload. This is not the previous cold/edit/repair sequence, and no old timing samples are copied into the new matrix.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_history_matrix.py all \
  --out /tmp/opencode/notist-history-matrix-NEW \
  --history /tmp/opencode/xiv-history-20261102 --runs 5 --cpu 8
```

`prepare`, `build`, `validate`, `run`, `probes`, and `report` also run separately. `measure` resumes validation, timing, probes, and reporting after builds finish. Use a new result directory when conditions change; completed samples can resume under the original settings.

## Workloads

| Workload | Complete HTML count | Purpose |
| --- | ---: | --- |
| xiv | 51,739 | Original complete pinned XIV DB production templates and the three historical Python script variants |
| literal | 521 | No notc imports: literal 13-field detail bodies, directory, navigation, real templates |
| shared | 521 | Same records, actual 13-field shared component graph, namespace/member/wildcard imports and re-exports |
| private | 521 | Same records and components, but a distinct module graph identity for every detail page; no first-build cross-page compilation reuse |
| wide | 521 | Four 64-function label libraries, a wildcard facade, eight real 32-row layouts; all 256 functions actually used in the full site |

Each fictional fixture contains 512 details, eight 64-record catalogs, a home page, styles, and full navigation. All use syntax supported by the earliest version. In the shared/private cases, integers are explicitly formatted into String arguments at the input boundary instead of using the later `str` builtin. Bool branches remain. The wide case accepts display strings. No `for`, array methods, later Data API, or private implementation behavior is backported to historical engines. These compatibility fixtures are a **new workload**, not the prior module-site timing results relabeled.

XIV's prepare hook returns most page sources as virtual documents. The fictional sites have physical page/module files. Their observed physical-input sets therefore differ substantially; this is intentional workload coverage, not a reason to compare per-page efficiency across different workloads.

The two Python optimization checkpoints only change XIV. Their other workload inputs and engine binaries are identical and act as controls; small timing variation there is not an optimization result. comemo off and compile are ablations of the same commit, not independent historical commits.

## Binaries and preservation

The earlier batch's historical **release binaries and source snapshots**, not timings, are reused only after matching their SHA-256, independent target provenance, and exact Rust toolchain. Ordinary and instrumented artifacts remain separate. The archived Handlebars artifact has no recovered complete uncommitted source and is a supplemental reference. Early Tera releases carry the same historically preexisting publication-path patch as before.

The final checkpoint is a new build of the committed HEAD captured by `prepare`; any uncommitted host policy in the main checkout is recorded in `checkout-start.diff` and excluded. Builds and instrumentation operate exclusively on result-directory copies. Current ordinary/probe builds have separate Cargo target directories to avoid cross-snapshot mtime reuse.

All resource versions use the benchmark's identical explicit request budget of 1,000,000, still through normal resource authorization, revision checks, reads, and accounting. This does not modify production defaults, consumer files, dependency locks, or the worktree's existing changes.

## Measurement and correctness

- Each engine/workload warms up completely before five rotating/reversed rounds of fresh-process `notist build`; timing includes full filesystem-backed SSG and transactional publication, including old-output cleanup. Same output path within each workload. Not a same-process hot-cache comparison.
- CPU affinity is fixed; frequency is not locked. Warm filesystem, no fsync. wall/user/system and wait4 RSS recorded. RSS is not simultaneous process-tree total.
- No compilation, HTML parsing, or output hashing runs concurrently with measured CLI processes.
- Every engine/workload is exhaustively checked for records, every field/row, page count, all item routes, and local links. Tera and later outputs must have identical full paths and per-file SHA-256; Handlebars/Tera must have the same HTML parser events and identical non-HTML bytes.
- A separate single probe per Tera-or-later cell verifies output equality and deterministic compile/bind actual-work counts. It is for attribution, not a multi-sample performance ranking. Times are inclusive.
- One fresh CLI may already reuse shared module compilation across pages; private module identities deliberately prevent that reuse. No-import controls must have zero module compilation work.

Results retain `manifest.json`, binary/source copies, fixed-input fingerprints, all stdout/stderr, `validation.json`, `validation-complete.json`, `runs.json`, `probes.json`, `summary.json`, `summary.csv`, and `report.md`. `dashboard.html` is a self-contained interactive grouped-bar chart; `history-wall.svg` and `history-rss.svg` contain the complete unscrolled static charts. The chart template is `tools/history_matrix_chart.html`.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools -p 'test_*.py' -v
```
