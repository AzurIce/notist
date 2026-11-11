# Parser merge: icexiv-db profile

## Results

Evidence: `/tmp/opencode/notist-parser-merge-20261111-verified`.
Compared `119c17d` with `13fafae` (merged parser plus compatibility fixes).
The later snapshot/test/tool commits do not change the measured runtime.

| Measurement | Before | After | Change |
| --- | ---: | ---: | ---: |
| Full build, median of 5 clean runs | 6.4413 s | 6.3325 s | -1.69% |
| Full build, min–max | 6.3866–6.4548 s | 6.2884–6.3462 s | |
| Peak RSS, median | 2.686 GiB | 2.688 GiB | effectively unchanged |
| Parse, median of 3 timing captures | 248.26 ms | 188.29 ms | -24.15% |
| Lowering, median of 3 timing captures | 93.48 ms | 97.62 ms | +4.42% |
| Site analysis, median of 3 timing captures | 1.0394 s | 0.9852 s | -5.21% |

All five interleaved pairs favor the candidate. The parser improvement is real
on this workload, but parsing accounts for only about 4% of the baseline build.
Other stages also fluctuate, so the entire wall-time delta must not be attributed
to parsing alone. No meaningful peak-memory reduction was observed.

The complete output inventory contains 78,004 byte-identical files, including
51,739 HTML pages. `check_site.py` verifies 51,225 item pages, 513 catalog pages
and all local HTML links. The current workspace passes 954 Rust tests; the
comparison driver has five unit tests.

### CPU hotspots

Both post-merge `cycles:u` recordings have zero lost samples. Flat self-cost
reports, aggregated across threads and inherited children, show:

| Symbol | After, recording 1 | After, recording 2 |
| --- | ---: | ---: |
| `malloc_consolidate` | 26.68% | 26.02% |
| `unlink_chunk.isra.0` | 11.39% | 11.46% |
| `std::path::compare_components` | 4.57% | 4.07% |
| `__libc_malloc2` | 4.16% | 4.25% |
| `_int_free_chunk` | 3.52% | 3.57% |

Expanded stacks associate a prominent allocator cost with
`notist_ssg::build::render_templates` allocating chunk result vectors through
`Executor::map_chunks`. This identifies an investigation target, not proof of
an allocator defect or a measured optimization opportunity of equal size.
Path comparison/reference handling is another visible cost. The next useful
experiment would isolate template batching/allocation behavior before investing
further in parser tuning for this particular site.

Median stage wall durations after the merge are 1.745 s for rendering, 1.049 s
for evaluation, 0.985 s for analysis, 0.737 s for assets, 0.471 s for publication,
and 0.346 s for templates. These include their nested work; they must not be
added to nested spans or interpreted as CPU percentages. Allocator CPU may be
spread across parallel workers even when the stage's wall time is short.

### Hardware-counter caveats

The two `perf stat` runs per revision multiplex six counters at approximately
83% running time. User-mode instructions are 108.48–108.76 billion before and
107.60–107.71 billion after (about -0.9% at the midpoint). The first baseline
counter run is a wall/cycle outlier (9.39 s versus 6.50 s for the second), so its
cycle/cache deltas are not used as a clean performance claim. Clean timings,
instrumented stage captures, frame-pointer sampling and counter runs remain
separate datasets. Kernel execution and off-CPU waits are not sampled here.

### Evidence files

- `manifest.json`: revisions, input/source inventories, binary hashes, perf setup.
- `samples.json`, `summary.json`: uninstrumented measurements.
- `*-timings*.json`, `stage-samples.json`, `stage-summary.json`: three captures
  per revision and their stage medians.
- `*-perf-*.data`: raw profiles; `*-flat.txt`: compact symbol-level self cost;
  `*-self.txt`, `*-cumulative.txt`, `*-stacks.txt`: expanded stack evidence.
- `*-stat-*-stat.json`: raw multiplexed counter results.
- `output-inventory.json`, `logs/check-site.log`: full output and site checks.

## Reproduce

```sh
PYTHONDONTWRITEBYTECODE=1 nix shell nixpkgs#perf -c python3 tools/profile_parser_merge.py all \
  --before 119c17d --after 13fafae \
  --out /tmp/opencode/notist-parser-comparison
```

Requires the local `icexiv-db` consumer and permission to record perf events.
Each evidence directory must be new. Source archives, consumer
inputs, compiler/environment metadata, binary hashes, raw samples, complete
output hashes, timing reports and CPU profiles are retained together.

Individual actions can also be run separately on the same `--out` directory.
On this host,
`kernel.perf_event_paranoid=1` permits recording; `2` permits counting but
recording is rejected. Perf records `cycles:u` at 499 Hz with frame-pointer
stacks, including inherited child processes. Two separate `perf stat` runs
per revision record instructions, cycles, branches and cache events.
The optional `profile` action uses the pinned gperftools paths in the script
instead of perf; it is not part of the default `all` sequence.

## Measurement contract

- Compare committed `main` before the merge with the merged parser plus
  compatibility fixes. Uncommitted documentation is not included.
- Copy consumer inputs once; never build or publish in its checkout. Ignore
  dangling symlinks, which do not supply readable input files.
- Build independent release targets with identical settings and two compiler
  workers. Build frame-pointer variants separately for CPU sampling.
- Warm up each revision, then run five interleaved pairs in fresh processes
  with warm filesystem caches. The workload is the complete `notist build`,
  including the Python extension and unchanged-output publication checks.
- Check every generated file's bytes outside the timed region. Run the
  consumer's `scripts/check_site.py` after the comparison.
- Capture `--timings=json` three times per revision separately; inclusive stage durations can overlap
  across workers and must not be added as sequential wall time.
- Sample each frame-pointer executable twice with perf `cycles:u` at 499 Hz,
  including inherited Python children. Clean release wall-time results do not
  include sampling or timing instrumentation. Kernel CPU is not sampled.
- `wait4` records per-invocation user/system CPU and peak RSS. CPU includes
  waited-for children; RSS is the maximum process high-water mark, not the
  simultaneous sum of the Rust and Python processes.

## Compatibility checks

The merge retains prose coalescing, lazy mode-aware tokenization and in-place
native content parsing. Compatibility fixes preserve inline annotation context,
use byte bounds for containers/planned cells across lexical mode switches,
honor bounds in Code lookahead, parse formatted/nested link labels, and keep
multiline/opaque call payloads within their logical table cells. Failed-closer
caches apply only inside the interval actually scanned.

Invalid inline bodies diagnose blank lines without dropping the rest of their
content. Unclosed calls retain the remaining content under the incomplete call;
the malformed-call corpus snapshot explicitly records that recovery behavior.
