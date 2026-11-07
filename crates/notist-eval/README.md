# notist-eval

Bounded evaluation of checked Notist IR. Runtime content binding, presentation,
native/data calls, compile-time evaluation and the evaluation-local instance
cache live here. Content trees use `Node { content: NodeContent, attrs, ... }`:
`NodeContent::Expr` holds a checked expression (including calls), while
`NodeContent::Item` holds constructed content. `Item` remains a compatibility
alias for `Node`; the item payload is `ContentItem`.

Complete output contains no expressions. Unannotated expanded expressions
dissolve into their resulting forest; nonempty annotations retain an ordinary
`Ctor::Group`, including for empty results. An empty annotation is equivalent
to no annotation. Explicit `Ctor::Node` boundaries remain. Native/component
nodes retain concrete parameters, but no source-call/target pair or evaluation
flags. Recovered failures are `Ctor::Error` content with source metadata.

Execution/recovery frames are private to the engine (`src/frame.rs`). This
representation refactor does not replace the capture algorithm or promise a
performance improvement. Current logical budgets still include intermediate
execution boundaries; output-tree size alone is not total evaluation work.

## Dependency boundary

```text
notist-pipeline ──→ notist-eval ──→ notist-core
        └───────────────────────→ notist-core
```

- `notist-core::evaluation` owns the shared `CheckedProgram`, `InstanceRecord`,
  `TypedExpr` and content-expression models. Calls use `ExprKind::Call` for
  both data and content results; call children are expressions as well.
- `EvaluationContext` supplies definitions, execution mappings and the callback
  for checking a missing specialization. Pipeline implements it with its checker.
  Generic dispatch avoids adding a trait-object lookup to every constructor call.
- `EvaluationInputs` supplies explicit host resource capabilities. There is no
  parser, module loader, filesystem, HTML backend or publication dependency.
- `evaluate` and `run` read the supplied checked Item tree. `constant`, `unary`
  and `binary` support the checker's existing compile-time evaluation.

Pipeline retains its public `evaluate*` methods and re-exports `Limits`,
`Evaluation` and `EvaluationOutcome`. It merges source texts and runs ordered
document extension hooks around this engine; callers of this crate directly
receive the raw engine result, not those pipeline hooks.

## Tests

`cargo test -p notist-eval` exercises native/core-only embedding and runtime
internals without a pipeline dependency. Compiler/evaluator integration tests
remain in pipeline. Its dev-dependency enables the internal `test-support`
feature for the existing checkpoint oracle and summary assertions. Normal
builds do not enable that feature.

## Scenario benchmarks

The compiler-backed driver lives in `tools/eval-bench`, outside this crate's
dependency graph. See its README for checked fixtures, clean timing vs separate
allocation diagnostics, scaling axes and reproducible evidence collection.
Run `cargo bench -p notist-eval-bench --bench scenarios` for evaluator timing.
