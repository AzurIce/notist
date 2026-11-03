# Profile one pushed revision

This profiles the production code at one exact pushed commit, not an uncommitted
worktree or an old timing sample. All source instrumentation is confined to
disposable `--out/sources` snapshots; the consumer, dependency locks, SSG host
budgets, page enumeration and `site.data` model are unchanged.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_current.py prepare \
  --revision 214de5f --out /tmp/opencode/notist-current-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_current.py build \
  --out /tmp/opencode/notist-current-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_current.py run \
  --out /tmp/opencode/notist-current-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_current.py python \
  --out /tmp/opencode/notist-current-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_current.py trace \
  --out /tmp/opencode/notist-current-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_current.py report \
  --out /tmp/opencode/notist-current-profile-NEW
```

`--inputs` defaults to the verified input-batch acceptance directory. It supplies
the complete XIV DB site (51,739 HTML pages) and four complete 521-page workloads,
plus their full per-file SHA-256 inventories. A separate JSON/TOML positive
control verifies that the Data read, actual parse, and cache-hit probes are live.
The archived inputs must be unchanged before preparation; all copied inputs,
source snapshots and binaries are fingerprinted and checked again afterward.

## Measurement boundaries

- **normal**: unmodified default-feature release CLI; five fresh-process rounds.
- **coarse**: independent stage/module-profiling binary; three rounds.
- **detail**: independent nested timing binary; three rounds.

Every mode/workload first completes one warmup. The order rotates, CPU affinity
is fixed, and no other profiling/build worker runs concurrently. Filesystem
caches are warm; CPU frequency is not locked. Every full output is checked after
the measured child exits. Compilation, JSON aggregation and verification are
outside the timers. The three binary modes are kept separate; probe overhead
and feature/code-generation differences are visible, not subtracted from normal
samples as if they were known pure timer costs.

Coarse stage partitions are disjoint and checked to sum to that probe's CLI
total. Site-local teardown is the measured SSG total less its explicit phases.
The detailed copy times selected successful-return destructors explicitly,
preserving the relative drop order of those objects: template values, directory
maps, component maps, source index, template engine, document trees, Vault, and
retired/staging publication directories. This is diagnostic attribution, not a
proposed change to production object lifetimes.

## Detailed timing semantics

`current_profile_probe.rs` is a std-only thread-local sink copied into
notist-core only in the detailed snapshot. It emits call counts, inclusive wall
time, self wall time, maximum single-call time and direct caller/child edges.
Self time excludes instrumented children, including recursive calls with the
same name; it is not a sampled CPU self-time profile and includes uninstrumented
work and measurement overhead. The sum of named self times must equal the root
CLI span. Never add inclusive parents to their children.

Detailed scopes cover native/Markdown parse and lowering, Resolve, Shape,
Materialize, evaluator binding/presentation, module compile/bind misses, host
environment/config creation, bounded filesystem reads, shared template value
construction, and teardown. Module work counts must agree across both probe
binaries and every repetition. Missing Data calls on the five main workloads
are reported as zero only after the positive control detects four reads, two
actual parses and two parse-cache hits.

## Independent diagnostics

The Python extension cProfile run uses a PATH wrapper around the same Python
interpreter and unchanged `scripts/items.py`; it is a separate full CLI run,
output-checked like the others, and its overhead is not mixed into ordinary
timings. Raw `python.pstats` and complete function rows are retained.

`strace -f -c` independently counts selected file, close, read, write and directory
syscalls on the complete XIV and private-module workloads. These include child
processes. Trace elapsed time is never used as a performance result.

No RSS ranking is made: wait4 high-water marks can inherit the Python launcher's
pre-exec memory peak. The profiler records wall, user and system time only.

## Independent evaluator drill-down

After the main batch, the remaining evaluator hotspot can be investigated with
one additional diagnostic binary, without changing the primary measurements:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_evaluator.py build \
  --out /tmp/opencode/notist-current-profile-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_evaluator.py run \
  --out /tmp/opencode/notist-current-profile-NEW
```

This copies the detailed source snapshot again and wraps existing Item,
prototype, children, Invocation, TypedExpr, definition and checked-body clones,
plus execution, implementation lookup, content counting, structural validation,
value limits and origin expansion. It never omits those operations. Four full
521-page workloads each run one warmup and three interleaved repetitions of both
the original detail and additional evaluator binary (32 complete builds).
Outputs and module-work counts must agree. The extra high-frequency timing
overhead is measured against that same-batch detail control; supplementary
timings are not mixed into ordinary CLI results.

The primary manifest/integrity files describe their own 76-run batch; the
supplement has its own manifest, source/binary hashes, samples and summary.
`final-integrity.json` records the combined independent verification when both
batches have completed. The interactive `current_profile_chart.html` template
can be populated with the compact chart payload independently of measurement.

## Evidence

The result directory retains the exact commit, source and input hashes, build
logs, normal and probe binaries, instrumentation patches, every timing sample,
per-run complete output inventories, caller edges, Python cProfile and strace
evidence. `summary.json` contains ordinary distributions, separate probe means,
module counts, disjoint stage partitions and ranked detailed self times.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools -p 'test_*.py' -v
rustc --edition 2024 --test -O tools/current_profile_probe.rs \
  -o /tmp/opencode/current-profile-sink-tests
/tmp/opencode/current-profile-sink-tests
```
