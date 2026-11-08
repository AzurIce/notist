use notist_core::{
    data::{DataCache, ParsedDataCache},
    definitions::{FunctionId, FunctionInstance},
    diag::Diagnostic,
    evaluation::{CheckedProgram, ExprKind, Invocation, TypedExpr},
    inputs::{EvaluationInputs, InputError, InputRead},
    item::{Item, Value},
    registry::Registry,
    resource::ResourcePath,
};
use notist_eval::{EvaluationContext, EvaluationOutcome, Limits};
use notist_pipeline::{Analysis, Context, Pipeline};
use serde_json::{Value as Json, json};
use std::{cell::Cell, sync::Arc};

pub const KINDS: &[&str] = &[
    "ready_text",
    "native_text",
    "code_calls",
    "capture_uses",
    "replay_value",
    "replay_call",
    "loop",
    "strict_lets",
    "capture_dead_branch",
    "capture_dead_branch_pruned",
    "unused_content",
    "many_parameters",
    "recover_errors",
    "read_text",
    "json_uncached",
    "json_cached",
    "pending_reads",
    "on_demand_instances",
    "nested_expansion",
];

pub struct Inputs {
    pub bytes: Arc<[u8]>,
    pub pending: bool,
    pub json: bool,
    pub cache: Option<DataCache>,
}

impl EvaluationInputs for Inputs {
    fn file(&self, path: &ResourcePath) -> InputRead {
        let expected = if self.json {
            "/bench/data.json"
        } else {
            "/bench/data.txt"
        };
        if path.logical_path() != expected {
            return InputRead::Failed(InputError::AccessDenied);
        }
        if self.pending {
            InputRead::Pending
        } else {
            InputRead::Ready(self.bytes.clone())
        }
    }

    fn data_cache(&self) -> Option<&dyn ParsedDataCache> {
        self.cache
            .as_ref()
            .map(|cache| cache as &dyn ParsedDataCache)
    }
}

struct AuditedInputs<'a> {
    inner: &'a Inputs,
    reads: Cell<usize>,
}

impl EvaluationInputs for AuditedInputs<'_> {
    fn file(&self, path: &ResourcePath) -> InputRead {
        self.reads.set(self.reads.get() + 1);
        self.inner.file(path)
    }
    fn data_cache(&self) -> Option<&dyn ParsedDataCache> {
        self.inner.data_cache()
    }
}

struct AuditedContext<'a> {
    inner: &'a Context,
    specializations: Cell<usize>,
}

impl EvaluationContext for AuditedContext<'_> {
    fn has_mappings(&self) -> bool {
        self.inner.extensions().has_replacements()
    }
    fn registry(&self) -> &Registry {
        self.inner.registry()
    }
    fn resource_root(&self) -> &str {
        self.inner.resource_root()
    }
    fn target(&self, id: &FunctionId) -> FunctionId {
        self.inner.extensions().target(id)
    }
    fn specialize(
        &self,
        instance: &FunctionInstance,
        program: CheckedProgram,
    ) -> (CheckedProgram, Vec<Diagnostic>) {
        self.specializations.set(self.specializations.get() + 1);
        self.inner.specialize(instance, program)
    }
}

pub struct Fixture {
    pub id: String,
    pub kind: String,
    pub size: usize,
    pub module: String,
    pub document: String,
    pub context: Context,
    pub analysis: Analysis,
    concrete: Option<notist_core::source::Located<Item>>,
    empty_program: CheckedProgram,
    pub inputs: Inputs,
    pub expected_text: String,
    pub expected_errors: usize,
    pub expected_reads: usize,
    pub pending: bool,
    pub canonical: Vec<u8>,
    pub facts: Json,
}

fn repeat(text: &str, n: usize) -> String {
    text.repeat(n)
}

fn balanced_branch(n: usize) -> String {
    if n == 1 {
        "name".into()
    } else {
        format!(
            "({} + {})",
            balanced_branch(n / 2),
            balanced_branch(n - n / 2)
        )
    }
}

pub fn prepare(kind: &str, n: usize) -> Result<Fixture, String> {
    if n == 0 || n > 512 || (kind == "nested_expansion" && n > 24) {
        return Err(format!("invalid size {n} for {kind}"));
    }
    let mut module = String::new();
    let mut document = "#self.view()".to_owned();
    let mut expected_text = "x".repeat(n);
    let mut expected_errors = 0;
    let mut expected_reads = 0;
    let pending = kind == "pending_reads";
    let json_input = matches!(kind, "json_uncached" | "json_cached");
    match kind {
        "ready_text" | "native_text" => document = repeat("#text(text: \"x\")", n),
        "code_calls" | "on_demand_instances" => {
            module = format!(
                "fn leaf(value: String) -> InlineContent {{ [#text(text: value)] }}\nfn view() -> InlineContent {{ [{}] }}",
                repeat("#leaf(\"x\")", n)
            )
        }
        "capture_uses" => {
            module = format!(
                "fn view(value: String) -> InlineContent {{ [{}] }}",
                repeat("#text(text: value)", n)
            );
            document = "#self.view(\"x\")".into();
        }
        "replay_value" => {
            module = format!(
                "fn view() -> InlineContent {{ let piece = [#text(text: \"x\")]; [{}] }}",
                repeat("#piece", n)
            )
        }
        "replay_call" => {
            module = format!(
                "fn leaf() -> InlineContent {{ [#text(text: \"x\")] }}\nfn view() -> InlineContent {{ let piece = leaf(); [{}] }}",
                repeat("#piece", n)
            )
        }
        "loop" => {
            let values = (0..n).map(|i| i.to_string()).collect::<Vec<_>>().join(",");
            module = format!(
                "fn view() -> InlineContent {{ for i in ({values},) [#text(text: str(i))] }}"
            );
            expected_text = (0..n).map(|i| i.to_string()).collect();
        }
        "strict_lets" => {
            let bindings = (0..n)
                .map(|i| format!("let v{i} = \"x\"; "))
                .collect::<String>();
            module = format!(
                "fn view() -> InlineContent {{ {bindings} [#text(text: v{})] }}",
                n - 1
            );
            expected_text = "x".into();
        }
        "capture_dead_branch" | "capture_dead_branch_pruned" => {
            // Grow expression size without also growing parser/interpreter
            // stack depth linearly: that would confound capture cost with depth.
            let branch = balanced_branch(n);
            let condition = if kind == "capture_dead_branch_pruned" {
                "true"
            } else {
                "flag"
            };
            module = format!(
                "fn view(name: String, flag: Bool) -> InlineContent {{ [#text(text: if {condition} {{ name }} else {{ {branch} }})] }}"
            );
            document = "#self.view(\"x\", true)".into();
            expected_text = "x".into();
        }
        "unused_content" => {
            module = format!(
                "fn leaf(value: String) -> InlineContent {{ [{}] }}\nfn view() -> InlineContent {{ let unused = leaf(\"x\"); [ok] }}",
                repeat("#text(text: value)", n)
            );
            expected_text = "ok".into();
        }
        "many_parameters" => {
            let parameters = (0..n)
                .map(|i| format!("p{i}: String"))
                .collect::<Vec<_>>()
                .join(",");
            module = format!(
                "fn view({parameters}) -> InlineContent {{ [#text(text: p{})] }}",
                n - 1
            );
            document = format!(
                "#self.view({})",
                (0..n).map(|_| "\"x\"").collect::<Vec<_>>().join(",")
            );
            expected_text = "x".into();
        }
        "recover_errors" => {
            module = format!(
                "fn view() -> InlineContent {{ [{}] }}",
                repeat("#text(text: panic(\"expected bench failure\"))", n)
            );
            expected_text.clear();
            expected_errors = n;
        }
        "read_text" | "pending_reads" | "json_uncached" | "json_cached" => {
            let (body, path) = if json_input {
                (
                    "let data = data_expect<Data>(read_json(source)); let value = data_expect<String>(data_require<String>(data, \"name\")); [#text(text: value)]",
                    "/bench/data.json",
                )
            } else {
                (
                    "let value = read_text(source); [#text(text: value)]",
                    "/bench/data.txt",
                )
            };
            module = format!(
                "fn leaf(source: Path) -> InlineContent {{ {body} }}\nfn view() -> InlineContent {{ [{}] }}",
                repeat(&format!("#leaf(p\"{path}\")"), n)
            );
            expected_reads = n;
            if pending {
                expected_text.clear();
            }
        }
        "nested_expansion" => {
            module = "fn f0() -> InlineContent { [#text(text: \"x\")] }\n".into();
            for i in 1..=n {
                module.push_str(&format!(
                    "fn f{i}() -> InlineContent {{ [#f{}()] }}\n",
                    i - 1
                ));
            }
            document = format!("#self.f{n}()");
            expected_text = "x".into();
        }
        _ => return Err(format!("unknown scenario {kind}")),
    }
    let pipeline = Pipeline::default();
    let mut context = Context::default();
    if !module.is_empty() {
        let compiled = pipeline
            .compile_module("bench.not", "bench", "/bench/lib.notc", &module)
            .map_err(|d| format!("compile {kind}/{n}: {d:?}"))?;
        context = context
            .with_module(compiled)
            .map_err(|d| format!("install {kind}/{n}: {d:?}"))?;
    }
    let analysis = pipeline
        .analyze("/bench/doc.not", &document, &context)
        .map_err(|e| e.to_string())?;
    if !analysis.diagnostics().is_empty() {
        return Err(format!("analyze {kind}/{n}: {:?}", analysis.diagnostics()));
    }
    let mut fixture = Fixture {
        id: format!("{kind}/{n}"),
        kind: kind.into(),
        size: n,
        module,
        document,
        context,
        analysis,
        concrete: None,
        empty_program: CheckedProgram::default(),
        inputs: Inputs {
            bytes: Arc::from(if json_input {
                br#"{"name":"x"}"#.as_slice()
            } else {
                b"x".as_slice()
            }),
            pending,
            json: json_input,
            cache: (kind == "json_cached").then(DataCache::default),
        },
        expected_text,
        expected_errors,
        expected_reads,
        pending,
        canonical: vec![],
        facts: Json::Null,
    };
    if kind == "ready_text" {
        let EvaluationOutcome::Complete(result) = fixture.evaluate() else {
            return Err("ready text must complete".into());
        };
        fixture.concrete = Some(notist_core::source::Located::new(
            result.root,
            result.sources,
        ));
    }
    fixture.audit()?;
    Ok(fixture)
}

impl Fixture {
    pub fn root(&self) -> &Item {
        self.concrete
            .as_ref()
            .map(|v| &v.value)
            .unwrap_or_else(|| self.analysis.root())
    }
    pub fn program(&self) -> &CheckedProgram {
        if self.kind == "on_demand_instances" {
            &self.empty_program
        } else {
            self.analysis.checked_program()
        }
    }
    pub fn evaluate(&self) -> EvaluationOutcome {
        notist_eval::run(
            std::hint::black_box(self.root()),
            self.program(),
            self.concrete
                .as_ref()
                .map_or(self.analysis.sources(), |v| &v.sources),
            &self.context,
            Limits::default(),
            Some(&self.inputs),
        )
    }
    fn audit(&mut self) -> Result<(), String> {
        let inputs = AuditedInputs {
            inner: &self.inputs,
            reads: Cell::new(0),
        };
        let context = AuditedContext {
            inner: &self.context,
            specializations: Cell::new(0),
        };
        let result = notist_eval::run(
            self.root(),
            self.program(),
            self.concrete
                .as_ref()
                .map_or(self.analysis.sources(), |v| &v.sources),
            &context,
            Limits::default(),
            Some(&inputs),
        );
        if inputs.reads.get() != self.expected_reads {
            return Err(format!(
                "{}: wrong logical read count {}",
                self.id,
                inputs.reads.get()
            ));
        }
        if (context.specializations.get() != 0) != (self.kind == "on_demand_instances") {
            return Err(format!(
                "{}: unexpected specialization count {}",
                self.id,
                context.specializations.get()
            ));
        }
        let (nodes, text_bytes, errors, dependencies, origin_depth) = match &result {
            EvaluationOutcome::NeedResources(paths) if self.pending && paths.len() == 1 => {
                (0, 0, 0, paths.len(), 0)
            }
            EvaluationOutcome::Complete(result) if !self.pending => {
                let mut text = String::new();
                let mut nodes = 0;
                let mut origin_depth = 0;
                for item in result.root.descendants() {
                    nodes += 1;
                    if item.as_expr().is_some() {
                        return Err(format!("{}: pending node in complete result", self.id));
                    }
                    if let Some(origin) = &item.origin {
                        origin_depth = origin_depth.max(origin.expansion.len());
                    }
                    if item.ctor() == notist_core::item::Ctor::Text
                        && let Some(Value::Str(value)) = item.field("text")
                    {
                        text.push_str(value);
                    }
                }
                if text != self.expected_text || result.diagnostics.len() != self.expected_errors {
                    return Err(format!(
                        "{}: incorrect text / diagnostics: text={text:?}, diagnostics={:?}",
                        self.id, result.diagnostics
                    ));
                }
                let expected_dependencies = usize::from(self.expected_reads != 0);
                if result.dependencies.len() != expected_dependencies {
                    return Err(format!("{}: wrong dependencies", self.id));
                }
                (
                    nodes,
                    text.len(),
                    result.diagnostics.len(),
                    result.dependencies.len(),
                    origin_depth,
                )
            }
            _ => return Err(format!("{}: unexpected completion state", self.id)),
        };
        let cache = self.inputs.cache.as_ref().map(DataCache::stats);
        if let Some(cache) = cache
            && cache.parses != 1
        {
            return Err(format!("{}: cache not warmed exactly once", self.id));
        }
        self.canonical = canonical(&result)?;
        self.facts = json!({
            "input_nodes": self.root().descendants().count(), "checked_instances": self.program().instances.len(),
            "output_nodes": nodes, "text_bytes": text_bytes, "diagnostics": errors,
            "dependencies": dependencies, "logical_reads": inputs.reads.get(),
            "specializations": context.specializations.get(), "max_origin_depth": origin_depth,
            "cache_parses_after_prepare": cache.map(|s| s.parses),
            "input_json_bytes": if self.inputs.json { self.inputs.bytes.len() } else { 0 },
            "input_ir": ir_counts(Some(self.root()), &CheckedProgram::default()),
            "program_ir": ir_counts(None, self.analysis.checked_program()),
        });
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        let current = canonical(&self.evaluate())?;
        if current != self.canonical {
            return Err(format!("{}: complete semantic output changed", self.id));
        }
        if let Some(cache) = &self.inputs.cache
            && cache.stats().parses != 1
        {
            return Err(format!("{}: warm parse cache unexpectedly missed", self.id));
        }
        Ok(())
    }
}

/// Counts IR occurrences, not unique allocations or elements inside constant
/// data arrays. Checked instance bodies are visited once per program entry.
fn ir_counts(root: Option<&Item>, program: &CheckedProgram) -> Json {
    #[derive(Default)]
    struct Counts {
        expressions: usize,
        items: usize,
        calls: usize,
    }
    fn invocation(call: &Invocation, counts: &mut Counts) {
        counts.calls += 1;
        for (_, argument) in &call.arguments {
            expression(argument, counts);
        }
        if let Some(child) = &call.children {
            expression(child, counts);
        }
    }
    fn item(node: &Item, counts: &mut Counts) {
        counts.items += 1;
        if let Some(expr) = node.as_expr() {
            expression(expr, counts);
            return;
        }
        for child in node.children() {
            item(child, counts);
        }
    }
    fn expression(expr: &TypedExpr, counts: &mut Counts) {
        counts.expressions += 1;
        match &expr.kind {
            ExprKind::Array(values) => {
                for value in values {
                    expression(value, counts);
                }
            }
            ExprKind::Dict(values) => {
                for (_, value) in values {
                    expression(value, counts);
                }
            }
            ExprKind::Block(values, result) => {
                for (_, value) in values {
                    expression(value, counts);
                }
                expression(result, counts);
            }
            ExprKind::Unary(_, value)
            | ExprKind::Some(value)
            | ExprKind::Unwrap(value)
            | ExprKind::Ok(value)
            | ExprKind::Err(value) => expression(value, counts),
            ExprKind::Binary(_, left, right) => {
                expression(left, counts);
                expression(right, counts);
            }
            ExprKind::If(condition, yes, no) => {
                expression(condition, counts);
                expression(yes, counts);
                expression(no, counts);
            }
            ExprKind::MatchResult { subject, arms } => {
                expression(subject, counts);
                for arm in arms {
                    expression(&arm.body, counts);
                }
            }
            ExprKind::Call(call) => invocation(call, counts),
            ExprKind::Content(forest) => {
                for node in forest.iter() {
                    item(node, counts);
                }
            }
            ExprKind::For { subject, body, .. } => {
                expression(subject, counts);
                expression(body, counts);
            }
            _ => {}
        }
    }
    let mut counts = Counts::default();
    if let Some(root) = root {
        item(root, &mut counts);
    }
    for record in program.instances.values() {
        if let Some(body) = &record.body {
            expression(body, &mut counts);
        }
    }
    json!({"typed_expressions": counts.expressions, "content_items": counts.items, "calls": counts.calls})
}

pub fn canonical(outcome: &EvaluationOutcome) -> Result<Vec<u8>, String> {
    let value = match outcome {
        EvaluationOutcome::Complete(result) => {
            let sources: std::collections::BTreeMap<_, _> = result
                .sources
                .texts()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|(path, text)| (path, text.as_ref()))
                .collect();
            let root = serde_json::to_value(result.sources.wire(&result.root))
                .map_err(|e| e.to_string())?;
            json!({"complete": {"root": root, "diagnostics": result.diagnostics, "dependencies": result.dependencies, "sources": sources}})
        }
        EvaluationOutcome::NeedResources(paths) => json!({"need_resources": paths}),
    };
    serde_json::to_vec(&value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_scenario_is_compiler_checked_and_semantically_stable() {
        for kind in KINDS {
            for n in [1, 8] {
                let fixture = prepare(kind, n).unwrap_or_else(|e| panic!("{e}"));
                fixture.validate().unwrap();
                fixture.validate().unwrap();
            }
        }
    }
    #[test]
    fn preexpanded_baseline_preserves_the_full_native_result() {
        let ready = prepare("ready_text", 8).unwrap();
        let native = prepare("native_text", 8).unwrap();
        assert_eq!(ready.canonical, native.canonical);
        assert_eq!(ready.facts["output_nodes"], native.facts["output_nodes"]);
    }
    #[test]
    fn growing_dead_code_does_not_change_output_work() {
        for kind in [
            "capture_dead_branch",
            "unused_content",
            "many_parameters",
            "strict_lets",
        ] {
            let small = prepare(kind, 1).unwrap();
            let large = prepare(kind, 128).unwrap();
            assert_eq!(small.expected_text, large.expected_text);
            assert_eq!(small.facts["output_nodes"], large.facts["output_nodes"]);
            large.validate().unwrap();
        }
    }

    #[test]
    fn compile_time_branch_control_removes_unused_ir() {
        let small = prepare("capture_dead_branch_pruned", 1).unwrap();
        let large = prepare("capture_dead_branch_pruned", 128).unwrap();
        let dynamic = prepare("capture_dead_branch", 128).unwrap();
        assert_eq!(small.facts["program_ir"], large.facts["program_ir"]);
        assert!(
            dynamic.facts["program_ir"]["typed_expressions"]
                .as_u64()
                .unwrap()
                > large.facts["program_ir"]["typed_expressions"]
                    .as_u64()
                    .unwrap()
        );
        assert_eq!(dynamic.facts["output_nodes"], large.facts["output_nodes"]);
    }
}
