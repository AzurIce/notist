# Stage observation

`notist-observe` supplies shared, opt-in stage observation for Notist's core
pipeline, evaluator, HTML renderer, site builder and hosts. It has no terminal,
filesystem or server dependencies.

## Library usage

The `notist` facade exports `timings` and `progress`:

```rust,ignore
let (result, report) = notist::timings::capture(|| {
    let span = notist::progress::Span::stage("host.build", || "Build".into());
    let result = build();
    if result.is_ok() {
        span.finish();
    }
    result
});
let html = report.html();
let json = report.json();
let stage_totals = report.summary();
```

Core operations record their own stages; hosts do not need to duplicate them.
Stage keys are separate from display labels. Source/module/cache metadata lives
in span attributes. Normal completion of an operation does not imply absence of
source diagnostics. Hosts should attach their overall result to report attributes.
An evaluation requesting more prepared resources is not a failed evaluation.

`capture` composes with the current progress observer. Both scopes restore previous
state on unwind. With neither observation nor timing enabled, labels remain lazy
and spans do not read clocks or allocate. Timing collection adds overhead and is
intended for diagnosing builds, not replacing controlled benchmarks.

To instrument workers, capture `timings::context()` before spawning, then run the
worker inside `timings::with_context(context, || ...)`. Join all workers before
returning from the parent span and capture. Timing contexts are Send/Sync; span
guards remain thread-local. Terminal progress observers are not propagated.
Nested spans on each thread must close in stack order.

Reports contain relative monotonic nanosecond intervals, IDs, parent IDs, lanes,
statuses and attributes. Stage totals sum inclusive and self elapsed times.
Self time subtracts the union of direct child intervals clipped to the parent;
nested and parallel totals must not be interpreted as wall-time percentages or
CPU time. Cache hits have lookup spans with `cache=hit` and no fabricated parse or
compile-work intervals. `cache=shared_empty` denotes the empty import fast path.

## CLI reports

```sh
notist build --timings
notist build --timings=html,json
notist check document.not --timings=json
notist serve --timings
```

Finite commands write timestamped reports to `target/notist-timings/` relative to
the working directory, including on failure. Reports are separate from published
site files. HTML is standalone and works offline. The timeline is collapsible and
searchable, with stage totals and slow document/module lists.

Tables constrain long labels and stage names with ellipses and full-text hover
titles. The time axis uses compact, width-dependent ticks. Desktop layouts fit
the available width; narrow screens scroll inside each table, not the page.
Run `node crates/notist-observe/tests/timings-layout.mjs` with Chromium available
to check desktop/mobile overflow, deep nesting, long names and tick spacing.

Serve retains at most 20 completed reports in memory, including failures:

- `/_notist/timings`: latest completed build, automatically follows new reports.
- `/_notist/timings/<id>`: fixed build, unaffected by subsequent rebuilds.
- `/_notist/timings/<id>.json`: raw report.
- `/_notist/timings/history.json`: retained build IDs in completion order.

These endpoints return 404 without `--timings`; serve does not write reports to
disk. Reports include the trigger, changed files, full-rebuild scope, page count,
result and update latency from the first received watcher notification through
publication (including debounce and queueing). Initial builds have no file-change
latency. Browser notification, reload and painting are not measured. Reports can
contain source paths; expose a timings-enabled server only to trusted clients.
The long-running language server does not support `--timings`.
