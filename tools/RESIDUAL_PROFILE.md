# Residual-cost analysis

`profile_residuals.py` inspects the already frozen optimization snapshot. It does
not change production Rust, the previous timing cohorts, consumer inputs, the
dependency lock or any budget. All full workloads remain intact. These are
**after-only diagnostic probes**, not another before/after speedup comparison.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_residuals.py prepare --out /tmp/opencode/notist-residual-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_residuals.py build --out /tmp/opencode/notist-residual-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_residuals.py run --out /tmp/opencode/notist-residual-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_residuals.py report --out /tmp/opencode/notist-residual-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_residuals.py trace --out /tmp/opencode/notist-residual-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_residuals.py extend --out /tmp/opencode/notist-residual-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_residuals.py build --mode checker-probe --out /tmp/opencode/notist-residual-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_residuals.py run_checker --out /tmp/opencode/notist-residual-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_residuals.py allocator --out /tmp/opencode/notist-residual-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_residuals.py report --out /tmp/opencode/notist-residual-NEW
```

The default `--base` is the retained optimization profile directory. Reuse its
source/binary fingerprints, not its seconds. CPU affinity defaults to 8; each
probe/workload has one warmup and three measured processes. Every full process,
including strace and allocator diagnostics, must match the complete original
per-file inventory. Initial evidence is retained before adding the checker
cohort. No source snapshots are rewritten after measuring them.

- **IO probe**: ownership metadata, directory open/next/drop, manifest read/parse,
  output-directory planning, rooted open, both metadata checks, bounded bytes and
  sharing. The original directory-empty expression is decomposed into the same
  ordered operations, including its temporary destruction order.
- **Evaluator probe**: runtime/comptime calls, argument preparation, actual target
  body dispatch, result adaptation, checked-instance lookup, capture, boundary
  copies, origin expansion and budget walks. Logical work counters separately
  count final nodes, mounted nodes and descendant visits. Dense timers and
  per-visit counters add overhead: their milliseconds are not ordinary CLI
  seconds, and recursive inclusive metrics must not be added.
- **Checker probe**: uncached module work, syntax parsing and the remaining
  unchecked CodeBody clone. End-of-batch observers count cached graphs,
  environments and distinct checked stores/plans/record references. Arc addresses
  only deduplicate observed storage; they are not semantic cache keys. Counts
  do not measure bytes, peak allocation or RSS, or the process-wide comemo cache.
- **Ownership strace**: runs the previous coarse publication-check binary,
  preserving its instruction/allocator behavior as far as possible. Syscall gaps
  localize the delay; trace wall time is never a performance sample.
- **Allocator observer**: Linux/glibc-only `residual_opendir_probe.c` wraps opendir
  and observes malloc calls made inside it, forwarding to the same __libc_malloc.
  It records slow allocations' wall and current-thread CPU times. It does not
  change allocator configuration or trim policy. Existing LD_PRELOAD is refused;
  the observer's source/library hashes and command are retained. Interposition
  itself is diagnostic overhead, so this cohort is never used for speedups.

The final verifier checks frozen production sources, primary snapshots, new
snapshots, binaries, inputs, every output inventory, and the timing stack's Self
partition. Raw edges and per-process distributions remain available; never move
an unexplained phase cost into another phase and call it a performance gain.
