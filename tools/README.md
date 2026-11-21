# Tools

This directory holds the maintained evaluator benchmark driver and its evidence helpers:

- `eval-bench/`: compiler-backed evaluator scenario benchmark crate (`notist-eval-bench`). See `eval-bench/README.md` for scenarios, commands, and measurement boundaries.
- `run_eval_bench.py`: freezes sources, builds separate clean timing (`time`) and allocation diagnostic (`alloc`) binaries in isolated Cargo target directories, runs fresh processes, and verifies semantic equality outside measurement. Diagnostic runs never supply timing results.
- `profile_support.py`: shared evidence primitives (SHA-256 digests, deterministic `[size, digest]` inventories, UTF-8 JSON serialization, and isolated Cargo target directories).
- `test_eval_bench.py`, `test_profile_support.py`: lightweight contract tests that run without launching benchmark workloads.

Run the contract tests:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools -p 'test_*.py'
node --test web/tests/*.test.mjs
```

Do not add machine-dependent timing thresholds to the ordinary regression gate.
