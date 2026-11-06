# Profile the current IceXIV working tree

Unlike the historical profilers, `profile_icexiv.py` snapshots the **current
consumer files, including uncommitted and ignored generated inputs**. It does
not rerun preparation, fetch game data, change limits, omit icons, reduce the
page count, or write to the consumer's `dist` directory. The Notist binary is
built from the exact committed `HEAD`, not from uncommitted production edits.

Use a fresh absolute output directory outside both checkouts:

```sh
python3 tools/profile_icexiv.py prepare --out /tmp/opencode/icexiv-profile-NEW
python3 tools/profile_icexiv.py build --out /tmp/opencode/icexiv-profile-NEW
python3 tools/profile_icexiv.py run --out /tmp/opencode/icexiv-profile-NEW
python3 tools/profile_icexiv.py serve --out /tmp/opencode/icexiv-profile-NEW
python3 tools/profile_icexiv.py python --out /tmp/opencode/icexiv-profile-NEW
python3 tools/profile_icexiv.py trace --out /tmp/opencode/icexiv-profile-NEW
python3 tools/profile_icexiv.py memory --out /tmp/opencode/icexiv-profile-NEW
python3 tools/profile_icexiv.py ordered --out /tmp/opencode/icexiv-profile-NEW
python3 tools/profile_icexiv.py report --out /tmp/opencode/icexiv-profile-NEW
```

## Measurement boundaries

- Normal: default-feature release binary, debug symbols enabled, five measured
  fresh processes after one warmup. Output is absent on the first normal warmup;
  all measured rounds compare identical existing output during publication.
- Coarse: independent profiling-feature binary, three measured processes and a
  warmup, timing major SSG stages, transport and destruction boundaries.
- Detail: independent profiling-feature binary, three measured processes and a
  warmup, adding environment, checking, evaluation and Data call-tree probes.
- Serve: the **normal** binary, one initial build followed by three same-content
  file-write rebuilds in the same process. This uses the actual `Updated` timer.
  The input write happens only inside the copied site and preserves its bytes.
- All modes retain normal CPU affinity and template worker selection (up to
  eight workers). Pinning to one CPU would change the real parallel workload.
  Filesystem caches are warm; CPU frequency and other system activity are not
  controlled. No profiling jobs are intentionally run concurrently.

`serve`'s displayed timer starts before target assembly and stops before
replacing the previous in-memory artifact. It includes Vault teardown but not
filesystem publication, previous artifact teardown, watcher debounce, watcher
registration, HTTP transfer or browser rendering. CLI process wall time has a
different boundary and must not be directly equated to `Updated` milliseconds.

The detailed timing sink records **wall time**, not sampled CPU time. Named Self
times partition the root, while inclusive parents overlap children. Template
workers are enclosed by a main-thread wait span; no per-worker timing is silently
discarded or incorrectly added into the wall partition. High-frequency recursive
probes add substantial overhead. Never substitute their seconds for normal
performance or treat an inclusive recursive total as a share of elapsed time.

The Python cProfile, syscall trace and 20-ms `/proc/<rust-pid>/status` RSS sampler
are independent diagnostic runs, not performance samples. RSS excludes the Python
extension, can miss peaks between samples, and is not allocator accounting.

The optional `ordered` supplement copies the frozen coarse snapshot and removes
its seven explicit SSG destructor timers. Original automatic destruction order
is then preserved. Total SSG teardown is derived by subtracting the disjoint
explicit stages from `ssg.total`; individual objects are not blamed for allocator
consolidation/trim work that can move between adjacent drop boundaries. It has
its own source/binary hashes, one warmup and three diagnostic rounds, and does
not replace or rewrite any primary cohort.

Every successful CLI build is validated against a complete per-file SHA-256
output inventory, outside the measured child interval. Serve checks its actual
page count; its preview HTML is not byte-compared with static output because the
serve host can inject preview-specific content. Input/source/binary fingerprints
and raw instrumentation patches are retained. Do not change a source snapshot
after compilation or reuse a result directory for a different input cohort.
