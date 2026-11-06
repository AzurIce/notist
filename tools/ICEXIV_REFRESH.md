# Re-profile the current dirty workspace

`profile_icexiv_refresh.py` is a measurement-only analysis tool, not another
optimization experiment. It never changes production source or the real
consumer. It includes tracked modifications and untracked source files, unlike
the older HEAD-only `profile_icexiv.py`.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_icexiv_refresh.py prepare --out /tmp/opencode/icexiv-refresh-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_icexiv_refresh.py build --out /tmp/opencode/icexiv-refresh-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_icexiv_refresh.py run --out /tmp/opencode/icexiv-refresh-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_icexiv_refresh.py serve --out /tmp/opencode/icexiv-refresh-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_icexiv_refresh.py diagnostics --out /tmp/opencode/icexiv-refresh-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_icexiv_refresh.py allocations --out /tmp/opencode/icexiv-refresh-NEW
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_icexiv_refresh.py report --out /tmp/opencode/icexiv-refresh-NEW
```

## Frozen source and input provenance

- `current`: all tracked and non-ignored untracked files from the workspace,
  preserving dirty Rust/Cargo/module/test changes. A before/after hash check
  refuses files changing during the freeze. A complete tree hash identifies the
  snapshot; HEAD alone does not describe it.
- `before`: exact source revision from the previous 2026-11-04 IceXIV profile.
  It is rebuilt with the same compiler and release environment as `current`.
- `coarse` / `detail`: independently instrumented copies of `current`.
- Consumer: full live `icexiv-db` inputs, including ignored generated inputs,
  private `.data` and all icons, copied outside both checkouts. If these exactly
  match the prior input inventory, the prior complete output SHA-256 inventory
  is also used as an independent expected result.

`manifest.json` retains source/input/binary/reproducer hashes, the workspace
diff/status, compiler, affinity and build flags. No older result files are
overwritten. Build dependencies share a fresh external Cargo target; local
workspace artifacts are cleaned between source paths to avoid archive-mtime
freshness errors. Normal binaries do not enable profiling features.

## Cohorts

1. Normal: one warmup per version, then six fresh-process rounds per version,
   alternating AB/BA order. Existing identical output is retained, so measured
   publication is a full bounded content comparison. The first warmup's initial
   publication is not mixed into these results.
2. Diagnostics: coarse/detail each one warmup and three rounds. Coarse stages
   preserve the production SSG consume-during-HTML lifetime and automatic Drop
   order; no temporary object is explicitly moved to another release boundary.
3. Serve: four independent normal processes in before/current/current/before
   order, each with an initial build and three same-byte file-write rebuilds.
   This yields two initial and six rebuild observations per version. The actual
   production `Updated` timer is recorded, not inferred from CLI wall time.
4. RSS: separate AB/BA normal processes with 20-ms Rust-only `/proc` RSS samples.
   Kernel wait4 high-water marks are recorded too, conservatively including the
   largest waited-for descendant rather than summing process memory.
5. Additional independent current-only syscall trace and Python cProfile.
6. Optional Linux/glibc allocation API observer: two AB/BA runs per normal
   version. It forwards malloc/calloc/realloc/free to glibc and enables counts
   only for the exact Rust executable, not the Python child. Requested bytes
   are traffic, not live memory; aligned APIs and libc-internal calls are not
   counted. Source/library/compiler fingerprints are retained. Interposition
   overhead and its wall time are never used as performance or RSS results.

No compile/profile worker from this task intentionally runs concurrently with
timed cohorts. Affinity and the normal up-to-eight template workers remain
unchanged. Filesystem caches are warm, CPU frequency and other system activity
are not locked, and slow samples are retained.

Every successful CLI run matches the complete 51,739-page output inventory,
outside its measured child interval. The consumer's full static-link checker is
also run. Serve observations validate reported page count, not every HTTP byte;
they are timing probes rather than a full nine-edit semantic/watch regression.

## Timing interpretation

CLI wall covers subprocess launch through wait4, including extension processes,
publication, destructors and exit. `Updated` covers target assembly, but not disk
publication, watcher debounce, old-artifact replacement, HTTP or the browser.

The main-thread timing sink reports nested inclusive and Self **wall** time, not
sampled CPU time. Recursive inclusive totals overlap and must never be added.
Main-thread template timing encloses worker joins, preserving the wall partition.
`ssg.total` minus explicit phases includes automatic SSG teardown; `cli.assemble`
minus `ssg.total` includes Vault destruction and host work. With trees now consumed
inside HTML rendering, some destruction belongs to that HTML loop rather than
the function return: apparent phase migration is not necessarily a regression.

Instrumented seconds, RSS diagnostic wall time, strace and cProfile are never
pooled with ordinary performance samples or used as before/after speedups.
