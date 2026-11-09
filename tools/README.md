# Profiling tools

New experiments import evidence primitives from `profile_support.py`, not from
an earlier experiment. The module provides SHA-256 file digests, deterministic
`[size, digest]` inventories, the existing JSON evidence format, and a separate
Cargo target directory for each revision/feature label.

Keep workload-specific setup, instrumentation and analysis in the experiment.
Historical script entry points and evidence formats remain available; extraction
of shared helpers does not rewrite old reports or recorded measurements.

`run_eval_bench.py` builds its clean and allocation variants from the frozen
source copy, in separate targets. Diagnostic runs never supply timing results.
Do not add machine-dependent timing thresholds to the ordinary regression gate.

Run the lightweight contracts without launching profiling workloads:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools -p 'test_*.py'
node --test web/tests/*.test.mjs
```

Full-workload experiments additionally need their documented frozen inputs and
output inventories. These are not replaced by synthetic fixtures in CI.

`profile_path_reuse.py` compares a frozen candidate with a `profile_hotspots`
reference from the same base revision. Use a fresh external `--out` and pass
`--reference` for each action: `prepare`, `build`, `measure`, `allocations`, then
`verify`. It copies the workload independently, alternates clean A/B run order,
and checks every output against the reference. Allocation probes are separate
frame-pointer builds and do not contribute to clean timing results.
