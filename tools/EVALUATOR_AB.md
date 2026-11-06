# Evaluator A/B protocol

`profile_evaluator_ab.py` copies the previously fingerprinted complete IceXIV
workload and committed Notist `85ab69e` into a fresh external directory. It never
rewrites either live checkout, production limits, input data or dependency locks.
Do not run other build/profile jobs concurrently with its timing phases.

```sh
python3 tools/profile_evaluator_ab.py prepare --out /tmp/opencode/evaluator-ab-NEW
python3 tools/profile_evaluator_ab.py build --out /tmp/opencode/evaluator-ab-NEW
python3 tools/profile_evaluator_ab.py semantic --out /tmp/opencode/evaluator-ab-NEW
python3 tools/profile_evaluator_ab.py run --out /tmp/opencode/evaluator-ab-NEW
python3 tools/profile_evaluator_ab.py diagnose --out /tmp/opencode/evaluator-ab-NEW
python3 tools/profile_evaluator_ab.py heap --out /tmp/opencode/evaluator-ab-NEW
python3 tools/profile_evaluator_ab.py serve_ab --out /tmp/opencode/evaluator-ab-NEW
python3 tools/profile_evaluator_ab.py report --out /tmp/opencode/evaluator-ab-NEW
python3 tools/profile_evaluator_ab_light.py /tmp/opencode/evaluator-ab-NEW
python3 tools/analyze_evaluator_ab.py /tmp/opencode/evaluator-ab-NEW
```

## Treatments

- `base`: untouched default-feature release implementation.
- `instance`: per-evaluation successful-instance cache for **only Type/no**
  comptime arguments. Data/float comptime values retain the old canonical key
  path. Actual transform targets remain part of the instance identity; errors
  and provisional checking records are not reused.
- `signature`: Invocation's immutable FunctionDef snapshot is an Arc instead of
  a Box; each initial call still constructs its own signature. Captured/cloned
  invocations can share it. A serde adapter preserves the old wire value without
  enabling new dependency features.
- `scope`: parent-linked immutable environment frames; lexical forks replace
  full map clones. Shadowing removal creates a tombstone. This is an environment
  experiment, **not** a complete closure-conversion or slot-machine rewrite.
- `present`: consuming map/collect replaces allocate/push in present, enabling
  Rust's Vec in-place collection specialization. Node order, recovery boundary,
  budget checks and output ownership remain unchanged.
- `combined`: all four treatments together. Independent gains need not add.

All six normal binaries use the same release flags and no profiling feature,
allocator wrapper or per-call timers. Each runs the core/pipeline/library unit
and integration tests in its own frozen source snapshot before measurement.
One full-process warmup precedes six measured rounds. A balanced Williams order
distributes treatments across positions and preceding treatments. CPU affinity
and normal template workers remain unchanged. Filesystem caches are warm; system
load and CPU frequency are not locked. Wall/user/system and faults are retained.
Pairing is by round, not a claim that execution environments are identical.
Bootstrap intervals on six paired rounds are descriptive, not proof of broad
statistical generality or gains on different workloads.

Every CLI run, including independent diagnostics, must match all 78,004 output
file hashes. Verification and compilation occur outside measured child time.
Source, binary, input and reproducer fingerprints are checked again afterward.
Shared Cargo targets retain third-party dependencies but clean all local
workspace packages between builds to avoid archive-mtime freshness mistakes.
Four additional fixtures compare complete public analysis/evaluation JSON across
all six normal variants, including captured shadowing/repeated mounts, recovered
errors, generic instantiation and structured-data reads. These are supplementary
to the unchanged unit/integration tests, not substitutes for budget/transform
coverage.

## Independent observations

The `base-probe` and `combined-probe` snapshots have coarse SSG scopes with the
original automatic SSG destruction order, plus a separate fixed-capacity
evaluator observer. Their feature/code-generation/allocator behavior differs
from normal binaries and **must not be used for primary A/B gains**.

- `off`: coarse stage timing; no evaluator samples. The observer allocator
  wrapper is still linked, so this is not the normal binary.
- `count`: all pages, exact interpreter call/work counts. Final output node
  counting happens only here, using an allocation-free recursive walk.
- `time`: three separate runs, sampling every 256th item page, every eighth
  catalog page, and the home page. Reports calls, inclusive and self wall time by
  page kind. Selection is deterministic and not random sampling; timer overhead
  and systematic page selection bias remain. No scaling of these times is
  substituted for ordinary process performance.
- `alloc`: all pages; forwards to the same System allocator while counting
  successful requested allocation/reallocation and deallocation events inside
  evaluator scopes. Fixed arrays and stack storage avoid allocating for metric
  bookkeeping. Events are charged to the **innermost active scope**, not to a
  sampled allocation stack. Requested bytes include realloc's full new size;
  freed bytes can belong to allocations made elsewhere. These are allocation
  traffic counts, **not peak live heap, unique bytes or RSS**. This observer does
  not include allocator-internal allocations or Python's separate process.
- `rss`: two independent ordinary-binary CLI runs per treatment, sampling only
  the Rust PID's VmRSS every 25ms. It can miss transient peaks and is not a live
  heap profile. Those runs do not participate in primary speedup distributions.
- `heap`: an additional Linux/glibc API interposer observes malloc/calloc/realloc/
  free of each ordinary Rust CLI, forwarding to the same __libc allocator. The
  Python child is explicitly excluded by executable path. This supplies a common
  whole-process allocation-traffic comparison for all six treatments, not live
  heap or a speedup measurement. Aligned entrypoints, libc-internal calls and
  allocations before constructor activation are not included. Requested bytes
  count the full new realloc size; frees count API calls, not unique objects.

Recursive inclusive times overlap. Only self times partition the observed root;
the parser verifies this identity for every sampled page kind. Empty present
calls, forest entries, final nodes and COW branch counters prevent confusing
function entries with unique or simultaneously live content nodes.

Neither this experiment nor the old wall-time probes establish native CPU stack
shares, allocator-internal consolidation time or full closure-conversion gains.
Keeping current semantics means repeated mounts still execute independently,
and checks, failures, effects, origins and logical resource budgets are not
omitted to manufacture a speedup.

The supplemental `serve_ab` cohort runs base and combined sequentially, each in
one ordinary serve process with an initial build and three same-content rebuilds.
It observes actual Updated milliseconds, not a CLI-minus-publication estimate.
Its small, unbalanced process order does not replace the six-round primary CLI
comparison. Preview page counts are checked; preview HTML is not byte-compared
with static HTML because the server can inject preview-specific content.

`profile_evaluator_ab_light.py` adds an independent base/combined stage-only
cohort after primary measurement. It copies the frozen normal sources, keeps
automatic SSG destruction order, and preallocates coarse metric/edge records
before the root CLI timer. It does **not** link the dense evaluator observer or
its global allocator. Each version has one warmup and three alternating measured
full builds; separate hashes, patches, samples and the exact supplement script
are retained in light.json / light-summary.json / light-reproducer.py. This
supplement never rewrites the primary manifest, source snapshots or timings.
