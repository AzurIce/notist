use notist::{
    Context, Data, DataCache, DataFormat, DataLimits, Evaluation, EvaluationInputs,
    EvaluationOutcome, InputRead, Limits, ParsedDataCache, Pipeline, ResourcePath, Type, Value,
};
use notist_core::data::DataFailure;
use std::{cell::Cell, sync::Arc};

struct Inputs {
    bytes: Arc<[u8]>,
    cache: DataCache,
    files: Cell<usize>,
    cache_lookups: Cell<usize>,
    parses: Cell<usize>,
    decodes: Cell<usize>,
}
impl Inputs {
    fn new(json: &str) -> Self {
        Self {
            bytes: Arc::from(json.as_bytes()),
            cache: DataCache::default(),
            files: Cell::new(0),
            cache_lookups: Cell::new(0),
            parses: Cell::new(0),
            decodes: Cell::new(0),
        }
    }
    fn assert_calls(&self, files: usize, decodes: usize) {
        assert_eq!(self.files.get(), files);
        assert_eq!(self.parses.get(), files);
        assert_eq!(self.decodes.get(), decodes);
        // Direct decoding must not even ask the host for its cache adapter.
        assert_eq!(self.cache_lookups.get(), files + decodes);
    }
}
impl EvaluationInputs for Inputs {
    fn file(&self, _: &ResourcePath) -> InputRead {
        self.files.set(self.files.get() + 1);
        InputRead::Ready(self.bytes.clone())
    }
    fn data_cache(&self) -> Option<&dyn ParsedDataCache> {
        self.cache_lookups.set(self.cache_lookups.get() + 1);
        Some(self)
    }
}
impl ParsedDataCache for Inputs {
    fn parse(
        &self,
        format: DataFormat,
        bytes: Arc<[u8]>,
        limits: DataLimits,
    ) -> Result<Data, DataFailure> {
        self.parses.set(self.parses.get() + 1);
        self.cache.parse(format, bytes, limits)
    }
    fn decode(&self, data: &Data, target: &Type, limits: DataLimits) -> Result<Value, DataFailure> {
        self.decodes.set(self.decodes.get() + 1);
        self.cache.decode(data, target, limits)
    }
}

fn evaluate(body: &str, inputs: &Inputs, limits: Limits) -> Evaluation {
    let pipeline = Pipeline::default();
    let context = Context::default()
        .with_module(
            pipeline
                .compile_module(
                    "page.not",
                    "view",
                    "/project/view.notc",
                    &format!(
                        "fn typed<T:Type>(value:T)->T{{value}} fn show()->InlineContent{{{body}}}"
                    ),
                )
                .unwrap(),
        )
        .unwrap()
        .with_resource_root("/project");
    let diagnostics = pipeline.check_module("view", &context);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{body}");
    let analysis = pipeline
        .analyze("/project/page.not", "#self.show()", &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let EvaluationOutcome::Complete(output) =
        pipeline.evaluate_with_inputs_and_limits(&analysis, &context, inputs, limits)
    else {
        panic!("unexpected pending input")
    };
    *output
}
fn text(output: &Evaluation) -> String {
    output
        .root
        .descendants()
        .filter_map(|node| match node.field("text") {
            Some(Value::Str(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}
fn assert_ok(output: &Evaluation) {
    assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
    assert_eq!(text(output), "ok");
}
const LOAD: &str = "let d=data_expect<Data>(read_json(p\"data.json\"));";

#[test]
fn direct_views_scalars_and_nested_options_bypass_the_cache_callback() {
    for (json, ty, expected) in [
        (r#"{"x":1}"#, "Data", "d"),
        ("null", "Unit", "()"),
        ("true", "Bool", "true"),
        ("7", "Int", "7"),
        ("1.5", "Float", "1.5"),
        (r#""shared""#, "String", r#""shared""#),
    ] {
        let mut ty = ty.to_owned();
        let mut expected = expected.to_owned();
        for depth in 0..3 {
            let inputs = Inputs::new(json);
            let check = if ty == "Unit" {
                "\"ok\"".into()
            } else if json == "null" {
                "match value{none=>\"ok\",some(v)=>\"bad\"}".into()
            } else {
                format!("if value==typed<{ty}>({expected}) {{\"ok\"}}else{{\"bad\"}}")
            };
            let output = evaluate(
                &format!(
                    "{LOAD}let value=data_expect<{ty}>(data_decode<{ty}>(d));[#text(text:{check})]"
                ),
                &inputs,
                Limits::default(),
            );
            assert_ok(&output);
            inputs.assert_calls(1, 0);
            assert_eq!(inputs.cache.stats().projections, 0);

            if depth > 0 {
                let inputs = Inputs::new("null");
                assert_ok(&evaluate(
                    &format!(
                        "{LOAD}let value=data_expect<{ty}>(data_decode<{ty}>(d));[#text(text:match value{{none=>\"ok\",some(v)=>\"bad\"}})]"
                    ),
                    &inputs,
                    Limits::default(),
                ));
                inputs.assert_calls(1, 0);
            }
            ty = format!("Option<{ty}>");
            expected = if json == "null" {
                "none".into()
            } else {
                format!("some({expected})")
            };
        }
    }
}

#[test]
fn lookup_operations_share_the_direct_and_cached_decode_policy() {
    for (ty, operation, expected, decodes) in [
        ("String", "data_require<String>(d,\"s\")", "\"shared\"", 0),
        (
            "Option<String>",
            "data_get<String>(d,\"s\")",
            "some(\"shared\")",
            0,
        ),
        (
            "Option<String>",
            "data_get<String>(d,\"missing\")",
            "none",
            0,
        ),
        ("Int", "data_require_index<Int>(a,0)", "1", 0),
        ("Option<Int>", "data_index<Int>(a,1)", "some(2)", 0),
        ("Option<Int>", "data_index<Int>(a,5)", "none", 0),
        (
            "Array<Int>",
            "data_require<Array<Int>>(d,\"a\")",
            "(1,2)",
            1,
        ),
        (
            "Option<Array<Int>>",
            "data_get<Array<Int>>(d,\"a\")",
            "some((1,2))",
            1,
        ),
        (
            "Array<Int>",
            "data_require_index<Array<Int>>(nested,0)",
            "(1,2)",
            1,
        ),
        (
            "Option<Array<Int>>",
            "data_index<Array<Int>>(nested,0)",
            "some((1,2))",
            1,
        ),
    ] {
        let inputs = Inputs::new(r#"{"s":"shared","a":[1,2],"nested":[[1,2]]}"#);
        let check = if ty == "Array<Int>" {
            "if value.len()==2{\"ok\"}else{\"bad\"}".into()
        } else if ty == "Option<Array<Int>>" {
            "match value{some(v)=>if v.len()==2{\"ok\"}else{\"bad\"},none=>\"bad\"}".into()
        } else {
            format!("if value==typed<{ty}>({expected}){{\"ok\"}}else{{\"bad\"}}")
        };
        assert_ok(&evaluate(
            &format!(
                "{LOAD}let a=data_expect<Data>(data_require<Data>(d,\"a\"));let nested=data_expect<Data>(data_require<Data>(d,\"nested\"));let value=data_expect<{ty}>({operation});[#text(text:{check})]"
            ),
            &inputs,
            Limits::default(),
        ));
        inputs.assert_calls(1, decodes);
        assert_eq!(inputs.cache.stats().projections, decodes);
    }
}

#[test]
fn direct_decode_errors_keep_the_callers_source_bound_view() {
    let json = r#"{"value":false}"#;
    let inputs = Inputs::new(json);
    let output = evaluate(
        r#"
let a=data_expect<Data>(read_json(p"a.json"));
let b=data_expect<Data>(read_json(p"b.json"));
let ignored=data_require<String>(a,"value");
let view=match data_expect<Option<Data>>(data_decode<Option<Data>>(b)){some(d)=>d,none=>panic("missing")};
let value=data_expect<Option<String>>(data_require<Option<String>>(view,"value"));
[#text(text:"unreached")]
"#,
        &inputs,
        Limits::default(),
    );
    inputs.assert_calls(2, 0);
    assert_eq!(inputs.cache.stats().parses, 1);
    assert_eq!(inputs.cache.stats().projections, 0);
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    let error = &output.diagnostics[0];
    assert_eq!(error.source.as_deref(), Some("/project/b.json"));
    assert!(error.message.contains("/value"));
    let source = output
        .sources
        .text(error.source.as_ref().unwrap().id())
        .unwrap();
    assert_eq!(source.as_ref(), json);
    assert_eq!(
        &source[usize::from(error.span.start())..usize::from(error.span.end())],
        "false"
    );
    assert!(!text(&output).contains("unreached"));
}

#[test]
fn direct_decode_limits_remain_hard_failures_without_a_cache_callback() {
    for data in [
        DataLimits {
            nodes: 1,
            ..Default::default()
        },
        DataLimits {
            depth: 1,
            ..Default::default()
        },
        DataLimits {
            value_bytes: 8,
            ..Default::default()
        },
    ] {
        let inputs = Inputs::new("7");
        let output = evaluate(
            &format!(
                "{LOAD}let value=match data_decode<Option<Int>>(d){{ok(v)=>\"value\",err(e)=>\"swallowed\"}};[#text(text:value)]"
            ),
            &inputs,
            Limits {
                data,
                ..Default::default()
            },
        );
        inputs.assert_calls(1, 0);
        assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
        assert!(output.diagnostics[0].message.contains("data projection"));
        assert!(!text(&output).contains("swallowed"));
    }
}

#[test]
fn arrays_and_optional_arrays_still_use_the_projection_cache_on_hits_and_errors() {
    for (json, ty) in [
        ("[1,2]", "Array<Int>"),
        ("[1,null]", "Array<Option<Int>>"),
        ("[1,2]", "Option<Array<Int>>"),
        ("[1,2]", "Option<Option<Array<Int>>>"),
        ("null", "Option<Array<Int>>"),
        ("[1,2]", "Array<Data>"),
        ("[true]", "Array<Int>"),
    ] {
        let inputs = Inputs::new(json);
        let check = if json == "[true]" {
            "match a{err(e)=>if data_error_kind(e)==\"type_mismatch\"{\"ok\"}else{\"bad\"},ok(v)=>\"bad\"}"
        } else {
            "match a{ok(v)=>\"ok\",err(e)=>\"bad\"}"
        };
        assert_ok(&evaluate(
            &format!(
                "{LOAD}let a=data_decode<{ty}>(d);let b=data_decode<{ty}>(d);[#text(text:if a==b {{{check}}}else{{\"bad\"}})]"
            ),
            &inputs,
            Limits::default(),
        ));
        inputs.assert_calls(1, 2);
        assert_eq!(inputs.cache.stats().parses, 1);
        assert_eq!(inputs.cache.stats().projections, 1);
    }
}

#[test]
fn decoding_does_not_charge_or_bypass_file_request_limits() {
    let inputs = Inputs::new("7");
    let output = evaluate(
        &format!(
            "{LOAD}let a=data_expect<Int>(data_decode<Int>(d));let b=data_expect<Option<Int>>(data_decode<Option<Int>>(d));let again=read_json(p\"data.json\");[#text(text:\"unreached\")]"
        ),
        &inputs,
        Limits {
            read_requests: 1,
            ..Default::default()
        },
    );
    inputs.assert_calls(1, 0);
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    assert!(
        output.diagnostics[0]
            .message
            .contains("resource request limit exceeded")
    );
    assert!(!text(&output).contains("unreached"));
}
