# notist-eval

Bounded evaluation of checked Notist IR. Runtime content binding, emission,
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

## Execution model

Content values are immutable construction plans. An opaque call or loop holds
shared checked code and a copy-on-write lexical environment. Saving or replaying
content does not substitute captured values into expressions, and does not
memoize the result of a mount. Sequential strict bindings and selected data
branches execute normally; unused content implementations remain opaque.

Binding resolves content splices while preserving opaque calls. Mounting emits
final `Node`s directly, with no execution-state tree or round-trip through
public IR. Aggregate overflow truncates an output suffix using the immutable
source boundary for recovery; there are no mutable-call backup checkpoints.
Concrete valid subtrees without execution mappings share their child storage.
Children, literal forests, calls and loop templates use shared COW containers;
editing checked input detaches it without changing another owner.

All calls enter one invocation engine and produce a value, reusable content or
an owned element draft. Immediate constructors finish their children and build
the final node without wrapping the result in a singleton content plan. Captured
content uses Empty/One/Many storage; empty plans allocate nothing and singleton
plans share the piece directly. Source metadata is separate from output payloads.

Arguments are evaluated and validated once, in checked-call order, into an owned
inline-small frame. Native operations borrow or consume its values by bound role;
Code transfers them to BindingId slots. Only constructors create a `DictBuilder`
and freeze persistent fields. Logical dictionary budgets do not depend on the
builder's physical capacity.

Logical budgets still include mounts which dissolve in the output. Loop work,
iterations and resource requests remain cumulative. Each mounted call owns its
step budget, and physical calls/loop nesting are capped. An unprepared resource
returns only `NeedResources`, never a partial output tree.

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

## Checked IR embedding

- Construct calls with `Invocation::new(instance, definition, arguments)`;
  argument names are shared `Text`. `arguments()` exposes the named wire/tooling
  projection and `checked_arguments()` exposes immutable bound positions.
- `set_arguments` and `with_definition` rebuild bindings without sorting or
  deduplicating arguments. `argument_name` returns `None` for an external invalid
  parameter position. Declarations and bound argument storage are not mutable
  independently. Textual wire format and its supplied argument order are unchanged.
- Construct implementation records with `InstanceRecord::new(definition)`.
  `FunctionInputs.parameters` aligns optional BindingIds with declaration positions;
  `children` is separate. `named_inputs()` is the explicit name projection.
  `with_definition` rebases input slots by name and rebuilds native roles.
- A host must supply a matching implementation kind and parameter layout for the
  selected definition. Inconsistent external records fail diagnostically after
  argument evaluation; they do not fall back to name-based execution. Existing
  source, binding and instance ownership validation remains required.

## Tests

`cargo test -p notist-eval` exercises native/core-only embedding and runtime
internals without a pipeline dependency. Compiler/evaluator integration tests
remain in pipeline. Its dev-dependency enables the internal `test-support`
feature for independent construction-count and output-invariant assertions.
Replay, rollback, source immutability and constrained-budget tests exercise
the same production engine. Normal builds do not enable that feature.

## Scenario benchmarks

The compiler-backed driver lives in `tools/eval-bench`, outside this crate's
dependency graph. See its README for checked fixtures, clean timing vs separate
allocation diagnostics, scaling axes and reproducible evidence collection.
Run `cargo bench -p notist-eval-bench --bench scenarios` for evaluator timing.
