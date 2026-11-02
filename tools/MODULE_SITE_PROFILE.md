# Module-rich complete site benchmark

This is a **fictional, deterministic componentized product site**, not the production XIV DB workload and not a claim about its speedup. The previous production workload has no notc imports and cannot meaningfully exercise module compilation caching.

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/profile_module_site.py all \
  --out /tmp/opencode/comemo-module-site-NEW \
  --pages 512 --private-pages 64 --runs 3 --cpu 8
```

Use a new output directory. `prepare`, `build`, `run`, and `report` can also run separately. An interrupted worker's incomplete output is rejected rather than mixed into valid samples. No consumers, dependency locks, engine implementation, or production limits are changed. The CLI package only adds a profiling-feature-gated example.

## Three workloads

- **shared**: 512 detail pages, eight 64-record catalog pages, a home page, a real Tera shell, styles, and validated navigation. All 13 record fields render through actual notc component calls. The library has formatting, labels, metadata, version, badge, a wildcard re-export facade, a detail component, eight section wrappers, and a catalog component. Namespace, member, and wildcard imports are used. Eighteen distinct modules are normally visited; the alternative version module is loaded only in the redirect case.
- **private**: 64 details and their catalog/home pages. Each detail imports a separate copy of the complete module graph under a unique source identity. This prevents first-build cross-page reuse; same-process rebuild reuse is still possible. Catalog/home pages do not import modules. Compare cache modes **within** this workload, not its absolute time against the 512-page site.
- **no-imports**: the full 512-record site as ordinary literal Markup, without any notc imports. This is a negative control for the previous workload's lack of module compilation work, not a comparison of two different language implementations' speed.

Records include varying titles, Chinese text, HTML-sensitive characters, integers, and booleans. Definitions are used to render the site rather than padded with large bodies of unused functions.

## Shared-workload edit sequence

Every step creates a fresh immutable input view and a new Vault, while retaining only the process-global computation caches:

1. `cold`: first whole-site assembly, computation caches empty.
2. `warm`: unchanged whole-site rebuild.
3. `body`: same-length version function body changes from stamp-A to stamp-B; every affected page must update, without invalidating unchanged name bindings.
4. `export`: append an unused export; wildcard dependents must invalidate, selective unaffected imports may reuse bindings; HTML must not change.
5. `local`: change only section 0's same-length body; exactly that eighth of detail pages must change.
6. `redirect`: a transitive re-export switches to a different module identity; the public stamp changes to C while the explicitly imported badge remains B.
7. `missing`: remove a required exported name; the assembly must fail with the expected missing-member diagnostic, not return stale success.
8. `missing-warm`: repeat that error to test error memoization.
9. `repaired`: restore the earlier valid input; the full output must match the body-edit result.
10. `evicted`: explicitly clear caches and rebuild; actual work must return to the first-build distinct-module/binding count, with identical output.

The other two workloads run only cold and unchanged warm rebuilds. Three modes (`off`, `compile`, `both`) run in **separate fresh processes of the same binary**, with rotated order. Bind/compile actual-work counts are asserted in addition to recording timings. Failed steps are correctness tests, never full-build speedup samples.

## Measurement boundary

The fixture is loaded into immutable `MemoryResources` outside the timer. Normal Vault resource reads, authorization, revision checks, and request accounting still run on every build; requests are explicitly and identically 1,000,000 in this host. The benchmark does not bypass the resource pipeline or cache whole documents, evaluation, templates, or site output.

Assembly time includes host cache maintenance, Vault creation, complete SSG analysis/evaluation/HTML/template/output-map assembly, and Vault destruction. Snapshot edits, Rust compilation, validation-copy writes, output destruction, HTML parsing, and SHA-256 are outside it. This is not CLI transactional publication or a disk benchmark. The SSG has no extra per-page timing probes; the existing optional module timers count compile/bind/link/install stages.

All timing workers finish before HTML parsing and hashing starts. Every output from every mode/repetition is checked for complete records, every field, catalog coverage, local links, expected stamps, source-change behavior, and exact byte equality with the corresponding other-mode output. Peak RSS is the `wait4` process high-water mark over the complete edit sequence, not an independent per-step peak or simultaneous process-tree sum. CPU affinity is fixed; frequency is not locked.

Raw JSON, stdout/stderr, fixture fingerprints, validation copies, and the release binary stay under the chosen output directory. `summary.json` and `report.md` contain aggregate results; per-step ranges and inclusive module times remain available in the JSON. `compile_work` is inside `compile`, and binding is inside linking; do not sum all stage times.
