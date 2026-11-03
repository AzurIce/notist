# Full-workload optimization verification

`profile_optimizations.py` compares the unmodified `214de5f` release artifacts
with a fingerprinted snapshot of the current production patch. Baseline timing
samples are **rerun**, not copied from the previous report. No commit or push is
performed by the tool.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_optimizations.py prepare \
  --out /tmp/opencode/notist-optimized-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_optimizations.py build \
  --out /tmp/opencode/notist-optimized-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_optimizations.py run \
  --out /tmp/opencode/notist-optimized-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_optimizations.py trace \
  --out /tmp/opencode/notist-optimized-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_optimizations.py report \
  --out /tmp/opencode/notist-optimized-profile-NEW
```

## Provenance

The source input is the exact archived revision plus `optimization.patch`, which
contains the tracked changes to `src`, `crates`, and Cargo files. Preparation
checks every production Rust source against the checkout. The dependency lock
must remain unchanged. Inputs, source snapshots, compiler version and binaries
are fingerprinted. The original baseline normal/detail artifacts must match the
earlier manifest and the compiler version must be identical. Their previous
seconds are never used as new samples.

New root integration regressions can be retained separately as
`regression-tests.patch`; crate unit tests are already part of the production
source patch. Successful preflight runs, if any, are recorded separately from
timing cohorts. All probe rewriting is limited to disposable source snapshots.

## Measurements

The five full workloads retain their original consumers, page enumeration,
entire body content, templates, site.data representation and dependency locks:
51,739 HTML files for XIV and 521 each for the four language/module workloads.
Every measured child is followed by complete per-file output verification.

Three publication states are deliberately separate:

- `same`: five rounds with byte-identical old output. Includes the new bounded
  verification of actual output bytes and exact directory entries, not only a
  timestamp or manifest-digest shortcut.
- `replace`: three rounds with one deliberately modified **output** index file.
  Input sources and consumers remain unchanged. This is a forced replacement
  control, not a simulation of the cost of arbitrary content edits. Staging,
  replacement and synchronous retired-directory cleanup remain timed.
- `first`: three rounds with no old output directory. The profiler removes only
  its own generated output before timing. It does not pretend that removing an
  old directory outside this timer is a faster rebuild; first builds and rebuilds
  are reported independently.

Each scenario/binary/workload has one warmup. Normal binary order alternates in
each round. The main comparison uses CPU affinity 8, warm filesystem caches and
fresh CLI processes. Frequency is not locked. Wall, user and system times are
recorded; RSS is not used.

The detailed probes have three rounds of identical rebuilds for all workloads
and a separate three-round XIV replacement control. The old and new probes are
independent binaries; their code generation and measurement overhead are not
subtracted from ordinary timings. Self times partition the root timer in the
single-core cohort. Preparation, stage timing and interpreter details remain
separate; `publish.compare` includes bounded output-file verification, whose
backend reads must not be mistaken for language Data reads.

A four-core XIV supplement measures only normal binaries, separately from the
single-core baseline comparison. Document input loading, hooks, evaluation and
publication remain ordered. Only prepared, immutable template work is eligible
for parallel rendering; the output merge and first error remain deterministic.
No threaded probe totals are claimed from the thread-local timing sink.

JSON/TOML positive controls verify four language Data reads and two actual
parses. Selected `strace -f -c` calls independently verify identical versus
replacement publication behavior; trace elapsed times are not performance
results. All old-directory cleanup stays synchronous when replacement occurs.

## Evidence

The directory retains source/binary/input manifests, production and optional
regression patches, build logs (including any pre-measurement probe correction),
individual samples, module-work counts, direct caller edges, full output
inventories, syscall evidence, summary distributions and final integrity checks.
The final report verifies that the current production patch still matches the
frozen measurement source. Do not interpret one changed optimization bundle as
an isolated A/B result for each individual edit.

`report` also writes `visualization-data.json` and a standalone `comparison.html`
from `tools/optimization_profile_chart.html`. Publication-state tabs use only
ordinary single-core medians; probe details and the four-core supplement remain
explicitly separate.

For after-only localization of publication checks, use a separate disposable
probe and evidence directory (not included in the primary speedup cohorts):

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_publication_checks.py \
  --out /tmp/opencode/notist-optimized-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_publication_checks.py \
  --out /tmp/opencode/notist-optimized-profile-NEW --report-only
```

This supplement retains its own source/binary fingerprints and complete output
checks. It cannot be used to replace primary samples or attribute the entire
optimization bundle to one edit. Parameter slots / instance IDs, an artifact
spool, and per-file incremental publication are not implemented by this bundle.

```sh
NO_COLOR=1 TERM=dumb cargo test --workspace --all-features --locked -- --test-threads=1
NO_COLOR=1 TERM=dumb cargo test -p notist --no-default-features --locked -- --test-threads=1
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools -p 'test_*.py' -v
```
