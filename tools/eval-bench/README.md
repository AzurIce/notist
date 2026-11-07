# Evaluator scenario benchmarks

Compile real `.notc` / Markup fixtures with pipeline, then call the production
`notist_eval::run` directly. No reverse dependency is added to notist-eval.
The driver excludes parsing, checking, fixture construction, JSON validation,
document hooks, HTML and filesystem IO from evaluator time.

## Reproduce

```sh
# Check all fixtures without timing; also works in debug builds.
cargo run -p notist-eval-bench -- --mode check

# Warm in-process timing, no allocation instrumentation.
cargo bench -p notist-eval-bench --bench scenarios -- --mode time --json /tmp/opencode/eval-time.json

# Independent diagnostic build. Its time is never used as a performance result.
cargo run --release -p notist-eval-bench --features alloc-metrics -- --mode alloc --samples 3 --json /tmp/opencode/eval-alloc.json

# Recommended: frozen evidence, separate binaries, three fresh timing processes.
PYTHONDONTWRITEBYTECODE=1 python3 tools/run_eval_bench.py --out /tmp/opencode/eval-bench-NEW
```

Use `--filter capture_dead_branch`, `--sizes 1,16,128`, `--samples 7`,
`--sample-ms 40` to narrow or repeat an experiment. Sizes are bounded to 1..512;
nested expansion separately uses depths 1/4/8/16 to stay below call-depth limits.
`--seed` controls deterministic shuffled case order. All raw samples are retained.
`--test` performs semantic checks rather than accidentally running benchmarks
when Cargo invokes the harness as a test target.

The timing command must not enable `alloc-metrics`. Its global allocator is the
ordinary allocator, with no allocation counters. Cargo's normal dependency graph
for this package must not enable notist-eval's `test-support` feature either.
`cargo bench --workspace --all-features` is not a clean timing configuration.

## Scenarios and interpretation

| Scenario | Size axis / question |
|---|---|
| ready_text vs native_text | Same full result: materialized native tree vs pending native constructors |
| code_calls | Many small parameterized Code calls |
| capture_uses | One Code call, many references to one captured parameter |
| replay_value / replay_call | Repeat one bound forest / repeat one opaque Code call recipe |
| loop | Deferred for, integer-to-string native operations and content mounting |
| strict_lets | Increasing strict local bindings, constant output |
| capture_dead_branch | Increasing *unselected* parameter-dependent expression size, constant output; balanced expression avoids conflating size with stack depth |
| capture_dead_branch_pruned | Same source-size axis but a compile-time true condition removes the unused branch; control, not a generally equivalent program |
| unused_content | Increasing body size of an unmounted opaque call, constant output; negative control |
| many_parameters | Increasing call/signature width, constant text/node count but more observable call fields |
| recover_errors | Expected failures/recovery, not falsely successful work |
| read_text | Shared immutable input bytes, one logical read per item |
| json_uncached / json_cached | Same input/results: per-read parse vs warmed parsed-data cache |
| pending_reads | Explicit Pending, no partial result exposed |
| on_demand_instances | Empty per-evaluation program; **Context checked-plan cache is warm**, not cold module compilation |
| nested_expansion | Increasing real Code call depth and output provenance |

Scale is not interchangeable across scenarios. Compare output nodes, text bytes,
logical reads, checked instances and IR counts alongside time. Except the explicit
on-demand case, preparation audits that no specialization callback occurs.

Every fixture checks expected text, diagnostic counts, reads and dependencies.
After every sample, an independent evaluation is compared byte-for-byte with the
prepared canonical **complete** root/diagnostics/dependencies/sources (or pending
paths), outside measurement. Identities, fields and origins are not normalized away.
The native/materialized pair additionally checks full-result equality.

## Measurement boundary

- Every timed iteration includes evaluator state, content construction and result
  destruction; no result clone or persistent result cache hides that cost.
- Compilation, cache warming, calibration, semantic checks and report serialization
  are outside samples. JSON cached input remains warm across runs in one process.
- Samples are batches calibrated to a target duration, interleaved in shuffled
  order. New processes reset application state, not OS frequency/background load.
- Allocation diagnostics count successful Rust global allocator requests, realloc
  requests and frees. Requested bytes are traffic, not copied bytes or RSS.
- Peak extra requested bytes and output-live bytes are incremental above existing
  inputs, program and warmed caches. They are **not** whole-process live heap/RSS.
  The allocation driver is single-threaded; it does not measure external allocators.
- These are warm synthetic evaluator scenarios, not predictions of full icexiv
  build time. Scaling correlations prioritize follow-up A/B/CPU/allocation tracing;
  they do not by themselves prove a particular helper caused every measured byte.
