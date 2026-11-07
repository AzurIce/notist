use notist::{
    Context, Data, DataCache, DataErrorKind as Kind, DataFormat, DataLimits, Evaluation,
    EvaluationInputs, EvaluationOutcome, InputError, InputRead, MemoryResources, ParsedDataCache,
    Pipeline, ResourcePath, Type, Value, Vault,
};
use notist_core::data::{DataFailure, parse};
use std::{cell::Cell, sync::Arc};

fn json(text: &str) -> Data {
    parse(DataFormat::Json, text.as_bytes(), DataLimits::default()).unwrap()
}
fn toml(text: &str) -> Data {
    parse(DataFormat::Toml, text.as_bytes(), DataLimits::default()).unwrap()
}
fn decode(data: &Data, t: Type) -> Value {
    data.decode(&t, DataLimits::default()).unwrap()
}
fn error(result: Result<Data, DataFailure>, kind: Kind) {
    assert!(
        matches!(result,Err(DataFailure::Error(ref e)) if e.kind()==kind),
        "{result:?}"
    );
}
fn context(module: &str) -> Context {
    Context::default()
        .with_module(
            Pipeline::default()
                .compile_module("content.not", "view", "/project/views/lib.notc", module)
                .unwrap(),
        )
        .unwrap()
        .with_resource_root("/project")
}
fn run(module: &str) -> Evaluation {
    let context = context(module);
    let pipeline = Pipeline::default();
    let diagnostics = pipeline.check_module("view", &context);
    assert!(diagnostics.is_empty(), "{diagnostics:?}\n{module}");
    let analysis = pipeline
        .analyze("/project/page.not", "#self.show()", &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    pipeline.evaluate(&analysis, &context)
}
fn expression(body: &str) -> Evaluation {
    run(&format!(
        "fn value() -> String {{ {body} }} fn show() -> InlineContent {{ [#text(text:value())] }}"
    ))
}
fn text(output: &Evaluation) -> String {
    output
        .root
        .descendants()
        .filter_map(|n| match n.field("text") {
            Some(Value::Str(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}
fn assert_text(body: &str, want: &str) {
    let out = expression(body);
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    assert_eq!(text(&out), want);
}

#[test]
fn closed_nodes_are_shared_ordered_and_strictly_decoded() {
    let root = json(r#"{"z":{"name":"语言"},"a":[null,false,0,1.0,1e0,""]}"#);
    assert_eq!(root.kind(), "Object");
    let z = root.require("z").unwrap();
    assert!(z.shares_document(&root));
    assert_eq!(z.path(), "/z");
    assert_eq!(
        decode(&z.require("name").unwrap(), Type::String),
        Value::Str("语言".into())
    );
    assert_eq!(
        root.keys()
            .unwrap()
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>(),
        ["\"z\"", "\"a\""]
    );
    let array = root.require("a").unwrap();
    assert_eq!(array.len().unwrap(), 6);
    assert_eq!(
        decode(&array.require_index(0).unwrap(), Type::Unit),
        Value::Unit
    );
    assert_eq!(
        decode(&array.require_index(1).unwrap(), Type::Bool),
        Value::Bool(false)
    );
    assert_eq!(
        decode(&array.require_index(2).unwrap(), Type::Int),
        Value::Int(0)
    );
    for i in [3, 4] {
        assert_eq!(
            decode(&array.require_index(i).unwrap(), Type::Float),
            Value::Float(1.0)
        );
        assert!(
            array
                .require_index(i)
                .unwrap()
                .decode(&Type::Int, DataLimits::default())
                .is_err()
        );
    }
    assert!(root.index(0).is_err());
    assert!(array.get("a").is_err());
    assert!(array.index(-1).is_err());
    assert_eq!(array.index(6).unwrap(), None);
    let reordered = json(r#"{"a":[null,false,0,1.0,1e0,""],"z":{"name":"语言"}}"#);
    assert_ne!(root, reordered);
}

#[test]
fn json_numbers_strings_duplicates_bom_and_malformed_inputs() {
    for source in ["9223372036854775808", "-9223372036854775809", "1e400"] {
        error(
            parse(DataFormat::Json, source.as_bytes(), DataLimits::default()),
            Kind::NumberRange,
        );
    }
    for (source, want) in [
        ("9223372036854775807", i64::MAX),
        ("-9223372036854775808", i64::MIN),
    ] {
        assert_eq!(decode(&json(source), Type::Int), Value::Int(want));
    }
    error(
        parse(
            DataFormat::Json,
            br#"{"a":1,"\u0061":2}"#,
            DataLimits::default(),
        ),
        Kind::DuplicateKey,
    );
    error(
        parse(
            DataFormat::Json,
            "\u{feff}null".as_bytes(),
            DataLimits::default(),
        ),
        Kind::Syntax,
    );
    error(
        parse(DataFormat::Json, &[255], DataLimits::default()),
        Kind::Utf8,
    );
    for source in [
        "01",
        "+1",
        "1.",
        "1e",
        "NaN",
        "Infinity",
        "[1,]",
        "{\"a\":}",
        "true false",
        "\"\\uD800\"",
        "\"line\nfeed\"",
        "{\"a\":1,}",
    ] {
        error(
            parse(DataFormat::Json, source.as_bytes(), DataLimits::default()),
            Kind::Syntax,
        );
    }
    assert_eq!(
        decode(&json(r#""\ud83d\ude00""#), Type::String),
        Value::Str("😀".into())
    );
    assert!(
        matches!(decode(&json("-0.0"),Type::Float),Value::Float(v) if v.to_bits()==(-0.0f64).to_bits())
    );
}

#[test]
fn nullable_decoding_and_lookup_keep_four_distinct_states() {
    for (key, want) in [
        ("missing", "missing"),
        ("null", "null"),
        ("empty", "value"),
        ("wrong", "error"),
    ] {
        assert_text(
            &format!(
                r#"let root=data_expect<Data>(parse_json("{{\"null\":null,\"empty\":\"\",\"wrong\":0}}"));
match data_get<Option<String>>(root,"{key}") {{ err(e)=>"error",ok(v)=>match v {{ none=>"missing",some(v)=>match v {{none=>"null",some(s)=>"value"}} }} }}"#
            ),
            want,
        );
    }
    assert_text(
        r#"let root=data_expect<Data>(parse_json("{\"n\":null}")); match data_require<Option<String>>(root,"n"){ok(v)=>match v{none=>"nullable",some(s)=>s},err(e)=>"bad"}"#,
        "nullable",
    );
    assert_text(
        r#"let root=data_expect<Data>(parse_json("{\"n\":null}")); match data_get<String>(root,"n"){ok(v)=>"wrong",err(e)=>data_error_kind(e)}"#,
        "type_mismatch",
    );
    let values = json("[1,null,2]");
    let ty = Type::Array(Some(Box::new(Type::Option(Box::new(Type::Int)))));
    let decoded = decode(&values, ty.clone());
    assert!(ty.accepts(&decoded));
    assert!(
        values
            .decode(
                &Type::Array(Some(Box::new(Type::Int))),
                DataLimits::default()
            )
            .is_err()
    );
}

#[test]
fn every_forbidden_target_is_rejected_even_for_empty_or_null_nodes() {
    for target in [
        Type::Never,
        Type::Dict,
        Type::Array(None),
        Type::Path,
        Type::ResourceTarget,
        Type::Type,
        Type::InlineContent,
        Type::BlockContent,
        Type::DataError,
        Type::Result(Box::new(Type::Int), Box::new(Type::String)),
    ] {
        for target in [
            target.clone(),
            Type::Option(Box::new(target.clone())),
            Type::Array(Some(Box::new(target))),
        ] {
            for source in ["null", "[]"] {
                assert!(
                    matches!(json(source).decode(&target,DataLimits::default()),Err(DataFailure::Error(e)) if e.kind()==Kind::UnsupportedTarget)
                );
            }
        }
    }
    for target in [
        "Never",
        "Dict",
        "Array",
        "Path",
        "ResourceTarget",
        "DataError",
        "Result<Int,String>",
        "Option<Path>",
        "Array<Never>",
    ] {
        let context = context(&format!(
            "fn show()->InlineContent{{let d=data_expect<Data>(parse_json(\"[]\"));let bad=data_decode<{target}>(d);[ok]}}"
        ));
        let ds = Pipeline::default().check_module("view", &context);
        assert!(
            ds.iter()
                .any(|d| d.message.contains("unsupported Data decode target")),
            "{target}: {ds:?}"
        );
    }
}

#[test]
fn data_functions_use_normal_generics_result_checks_and_comptime() {
    assert_text(
        r#"comptime data_expect<String>(data_require<String>(data_expect<Data>(parse_json("{\"name\":\"checked\"}")),"name"))"#,
        "checked",
    );
    assert_text(
        r#"let d=data_expect<Data>(parse_json("[4]")); let v=data_expect<Int>(data_require_index<Int>(d,0)); if v==4 && data_expect<Int>(data_len(d))==1 {"yes"}else{"no"}"#,
        "yes",
    );
    assert_text(
        r#"let d=data_expect<Data>(parse_json("[]"));match data_index<String>(d,-1){err(e)=>data_error_kind(e),ok(v)=>"wrong"}"#,
        "invalid_query",
    );
    assert_text(
        r#"let d=data_expect<Data>(parse_json("1"));if data_kind(d)=="Int" && d==data_expect<Data>(parse_json("1")){"same"}else{"wrong"}"#,
        "same",
    );
    assert_text(
        r#"let d=data_expect<Data>(parse_json("{\"a/b~\":false}"));match data_require<String>(d,"a/b~"){err(e)=>data_error_path(e),ok(v)=>"wrong"}"#,
        "/a~1b~0",
    );
    assert_text(
        r#"match parse_json("bad"){err(e)=>"fallback",ok(d)=>"wrong"}"#,
        "fallback",
    );
    let out = expression(
        r#"data_expect<String>(data_decode<String>(data_expect<Data>(parse_json("1"))))"#,
    );
    assert_eq!(out.diagnostics.len(), 1);
    assert!(out.diagnostics[0].message.contains("expected string"));
}

#[test]
fn toml_subset_keeps_declaration_order_and_rejects_dates_and_nonfinite() {
    let root = toml(
        "z=1\na={name=\"kept\", enabled=true}\nrows=[{v=1},{v=2}]\n[t]\nx=1.0\n[[more]]\nname='first'\n",
    );
    assert_eq!(
        root.keys()
            .unwrap()
            .iter()
            .map(|v| v.to_string())
            .collect::<Vec<_>>(),
        ["\"z\"", "\"a\"", "\"rows\"", "\"t\"", "\"more\""]
    );
    assert_eq!(
        decode(
            &root.require("a").unwrap().require("name").unwrap(),
            Type::String
        ),
        Value::Str("kept".into())
    );
    assert_eq!(root.require("more").unwrap().len().unwrap(), 1);
    for source in ["v=1979-05-27", "v=07:32:00", "v=1979-05-27T07:32:00Z"] {
        error(
            parse(DataFormat::Toml, source.as_bytes(), DataLimits::default()),
            Kind::UnsupportedValue,
        );
    }
    for source in ["v=nan", "v=+inf", "v=-inf"] {
        error(
            parse(DataFormat::Toml, source.as_bytes(), DataLimits::default()),
            Kind::NumberRange,
        );
    }
    assert!(parse(DataFormat::Toml, b"v=1\nv=2", DataLimits::default()).is_err());
    assert_text(
        r#"let d=data_expect<Data>(parse_toml("name='TOML'"));data_expect<String>(data_require<String>(d,"name"))"#,
        "TOML",
    );
}

const VIEW: &str = r#"
fn item(source:Path,id:String)->BlockContent{
 let root=data_expect<Data>(read_json(source));
 let record=data_expect<Data>(data_require<Data>(root,id));
 let name=data_expect<String>(data_require<String>(record,"name"));
 let description=match data_expect<Option<String>>(data_get<String>(record,"description")){some(v)=>v,none=>""};
 [ #heading<2>[#text(text:name)]

#text(text:description) ]
}
"#;
fn files(data: &str) -> MemoryResources {
    let mut files = MemoryResources::new("/project");
    files.insert("views/lib.notc", VIEW);
    files.insert("private/items.json", data);
    files
}
const PAGE: &str =
    "#import \"views/lib.notc\" as view;\n#view.item(p\"private/items.json\",\"42\")";

#[test]
fn record_body_is_static_private_and_shared_across_pages() {
    let mut vault = Vault::new(files(
        r#"{"42":{"name":"静态 <&>","description":"正文"},"secret":"NOT_PUBLIC"}"#,
    ));
    for page in ["one.not", "two.md"] {
        let out = vault.render_html(page, PAGE, Default::default()).unwrap();
        assert!(
            out.evaluation.diagnostics.is_empty(),
            "{:?}",
            out.evaluation.diagnostics
        );
        assert!(
            out.rendered.diagnostics.is_empty(),
            "{:?}",
            out.rendered.diagnostics
        );
        assert!(out.rendered.html.contains("静态 &lt;&amp;&gt;"));
        assert!(out.rendered.html.contains("正文"));
        assert!(!out.rendered.html.contains("NOT_PUBLIC"));
        assert!(out.rendered.used_components.is_empty());
        assert_eq!(out.evaluation.dependencies.len(), 1);
        let inspected = notist::json::item(&out.evaluation.root).to_string();
        assert!(!inspected.contains("NOT_PUBLIC"));
    }
    assert_eq!(vault.input_session().data_cache_stats().parses, 1);
}

#[test]
fn data_fail_keeps_exact_data_source_path_span_and_expansion() {
    let source = r#"{"42":{"name":123,"description":"ok"}}"#;
    let mut vault = Vault::new(files(source));
    let out = vault
        .render_html("page.not", PAGE, Default::default())
        .unwrap();
    assert_eq!(
        out.evaluation.diagnostics.len(),
        1,
        "{:?}",
        out.evaluation.diagnostics
    );
    let error = &out.evaluation.diagnostics[0];
    assert_eq!(error.source.as_deref(), Some("/project/private/items.json"));
    assert!(error.message.contains("/42/name"));
    let attached = &out.evaluation.sources[error.source.as_ref().unwrap()];
    assert_eq!(attached.as_ref(), source);
    assert_eq!(
        &attached[usize::from(error.span.start())..usize::from(error.span.end())],
        "123"
    );
    let item = out.evaluation.root.find(|n| n.is_error()).unwrap();
    let expansion = &item.origin.as_ref().unwrap().expansion;
    assert!(
        expansion
            .iter()
            .any(|l| l.source == "/project/views/lib.notc")
    );
    assert!(expansion.iter().any(|l| l.source == "/project/page.not"));
}

#[test]
fn input_errors_pending_and_capability_failures_use_distinct_channels() {
    struct Inputs {
        read: InputRead,
        calls: Cell<usize>,
    }
    impl EvaluationInputs for Inputs {
        fn file(&self, _: &ResourcePath) -> InputRead {
            self.calls.set(self.calls.get() + 1);
            self.read.clone()
        }
    }
    let module = r#"fn show(source:Path)->InlineContent{let v=match read_json(source){err(e)=>data_error_kind(e),ok(d)=>"ready"};[#text(text:v)]}"#;
    let context = context(module);
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("/project/page.not", r#"#self.show(p"data")"#, &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    for (input, want) in [
        (InputRead::Failed(InputError::NotFound), "not_found"),
        (InputRead::Failed(InputError::AccessDenied), "access_denied"),
        (InputRead::Failed(InputError::NotFile), "not_file"),
        (InputRead::Failed(InputError::Io("io".into())), "io"),
        (InputRead::Ready(Arc::from(b"1".as_slice())), "ready"),
    ] {
        let inputs = Inputs {
            read: input,
            calls: Cell::new(0),
        };
        let EvaluationOutcome::Complete(out) =
            pipeline.evaluate_with_inputs(&analysis, &context, &inputs)
        else {
            panic!()
        };
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        assert_eq!(text(&out), want);
        assert_eq!(inputs.calls.get(), 1);
        assert_eq!(out.dependencies.len(), 1);
    }
    for input in [
        InputError::Unavailable,
        InputError::TooLarge,
        InputError::CacheLimit,
        InputError::RequestLimit,
    ] {
        let inputs = Inputs {
            read: InputRead::Failed(input),
            calls: Cell::new(0),
        };
        let EvaluationOutcome::Complete(out) =
            pipeline.evaluate_with_inputs(&analysis, &context, &inputs)
        else {
            panic!()
        };
        assert_eq!(out.diagnostics.len(), 1);
        assert!(!text(&out).contains("fallback"));
    }
    let inputs = Inputs {
        read: InputRead::Pending,
        calls: Cell::new(0),
    };
    assert!(matches!(
        pipeline.evaluate_with_inputs(&analysis, &context, &inputs),
        EvaluationOutcome::NeedResources(_)
    ));
    assert_eq!(pipeline.evaluate(&analysis, &context).diagnostics.len(), 1);
}

#[test]
fn cache_hits_preserve_type_proofs_order_sources_and_hard_budgets() {
    let cache = DataCache::default();
    let bytes: Arc<[u8]> = Arc::from(b"[1,null,2]".as_slice());
    let limits = DataLimits::default();
    let one = cache
        .parse(DataFormat::Json, bytes.clone(), limits)
        .unwrap();
    let two = cache.parse(DataFormat::Json, bytes, limits).unwrap();
    assert!(one.shares_document(&two));
    let ty = Type::Array(Some(Box::new(Type::Option(Box::new(Type::Int)))));
    let Value::Array(a) = cache.decode(&one, &ty, limits).unwrap() else {
        panic!()
    };
    let Value::Array(b) = cache.decode(&two, &ty, limits).unwrap() else {
        panic!()
    };
    assert!(a.shares_storage(&b));
    assert!(ty.accepts(&Value::Array(a)));
    assert_eq!(cache.stats().parses, 1);
    assert_eq!(cache.stats().projections, 1);
    assert!(matches!(
        cache.decode(
            &one,
            &ty,
            DataLimits {
                value_bytes: 8,
                ..limits
            }
        ),
        Err(DataFailure::Limit(_))
    ));
    let tiny = DataCache::new(1, 1);
    assert!(matches!(
        tiny.parse(DataFormat::Json, Arc::from(b"1".as_slice()), limits),
        Err(DataFailure::Limit(_))
    ));
    for limits in [
        DataLimits { nodes: 2, ..limits },
        DataLimits { depth: 1, ..limits },
        DataLimits {
            source_bytes: 1,
            ..limits
        },
        DataLimits {
            value_bytes: 8,
            ..limits
        },
    ] {
        assert!(matches!(
            parse(DataFormat::Json, b"[1,2]", limits),
            Err(DataFailure::Limit(_))
        ));
    }
}

#[test]
fn raw_views_are_not_comptime_value_parameters_but_type_values_are_legal() {
    for value in [
        r#"data_expect<Data>(parse_json("null"))"#,
        r#"parse_json("null")"#,
        r#"some(data_expect<Data>(parse_json("null")))"#,
    ] {
        let context = context(&format!(
            "fn parameter<T:Type,v:T>()->String{{\"bad\"}} fn show()->InlineContent{{let value=parameter<{},{value}>();[bad]}}",
            if value.starts_with("some") {
                "Option<Data>"
            } else if value.starts_with("parse") {
                "Result<Data,DataError>"
            } else {
                "Data"
            }
        ));
        assert!(
            Pipeline::default()
                .check_module("view", &context)
                .iter()
                .any(|d| d.message.contains("comptime value arguments"))
        );
    }
    assert_text(
        r#"let d=data_expect<Data>(parse_json("null"));if data_kind(d)=="Null"{"type argument works"}else{"bad"}"#,
        "type argument works",
    );
}

#[test]
fn parse_and_projection_errors_rebind_sources_without_poisoning_cached_values() {
    let cache = DataCache::default();
    let limits = DataLimits::default();
    let bytes: Arc<[u8]> = Arc::from(br#"{"rows":[{"v":null}]}"#.as_slice());
    let root = cache.parse(DataFormat::Json, bytes, limits).unwrap();
    for name in ["/a.json", "/b.json"] {
        let root = root.clone().bind_source(
            name.into(),
            notist::SourceLocation {
                source: "view.notc".into(),
                start: 0,
                end: 4,
            },
        );
        let rows = root.require("rows").unwrap();
        let Value::Array(values) = cache
            .decode(&rows, &Type::Array(Some(Box::new(Type::Data))), limits)
            .unwrap()
        else {
            panic!()
        };
        let Value::Data(row) = &values[0] else {
            panic!()
        };
        let value = row.require("v").unwrap();
        let Err(DataFailure::Error(error)) = cache.decode(&value, &Type::String, limits) else {
            panic!()
        };
        let (source, text, span, _) = error.diagnostic_source().unwrap();
        assert_eq!(source, name);
        assert_eq!(&text[span], "null");
        assert_eq!(error.path(), "/rows/0/v");
    }
    // Only the materialized Array<Data> is memoized; scalar failures stay local.
    assert_eq!(cache.stats().projections, 1);
    let malformed: Arc<[u8]> = Arc::from(br#"{"broken":}"#.as_slice());
    for name in ["/a.json", "/b.json"] {
        let Err(DataFailure::Error(error)) =
            cache.parse(DataFormat::Json, malformed.clone(), limits)
        else {
            panic!()
        };
        let error = error.bind_source(
            name.into(),
            notist::SourceLocation {
                source: "read.notc".into(),
                start: 0,
                end: 1,
            },
        );
        assert_eq!(error.diagnostic_source().unwrap().0, name);
    }
    assert_eq!(cache.stats().parses, 2);
}

#[test]
fn large_projected_arrays_are_cached_shared_and_keep_nullable_types_across_functions() {
    let values = format!("[{}null]", "1,".repeat(10_000));
    let mut resources = MemoryResources::new("/project");
    resources.insert("values.json", values);
    resources.insert(
        "view.notc",
        r#"
fn identity<T:Type>(value:T)->T { value }
fn bag(values:Array<Option<Int>>)->InlineContent;
fn show(source:Path)->InlineContent {
 let root=data_expect<Data>(read_json(source));
 let values=data_expect<Array<Option<Int>>>(data_decode<Array<Option<Int>>>(root));
 [#self.bag(identity(values))]
}"#,
    );
    let mut vault = Vault::new(resources);
    let mut arrays = Vec::new();
    for page in ["a.not", "b.not"] {
        let analysis = vault
            .analyze(
                page,
                "#import \"view.notc\" as view;\n#view.show(p\"values.json\")",
            )
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{:?}",
            analysis.diagnostics()
        );
        let output = vault.evaluate(page, &analysis).unwrap();
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        let bag = output
            .root
            .find(|n| n.function_id() == Some(notist::FunctionId::new("/project/view.notc", "bag")))
            .unwrap();
        let Some(Value::Array(values)) = bag.field("values") else {
            panic!()
        };
        arrays.push(values.clone());
    }
    assert!(arrays[0].shares_storage(&arrays[1]));
    assert_eq!(arrays[0].len(), 10_001);
    assert_eq!(arrays[0].last(), Some(&Value::None));
    assert_eq!(vault.input_session().data_cache_stats().parses, 1);
    assert_eq!(vault.input_session().data_cache_stats().projections, 1);
}

#[test]
fn standalone_markup_never_wrappers_aliases_and_comptime_cannot_create_read_entries() {
    let context = context(
        r#"
fn load(source:Path)->Data { data_expect<Data>(read_json(source)) }
fn stop(source:Path)->Never { let value=load(source);panic("stop") }
fn show(value:String)->InlineContent { [#text(text:value)] }
"#,
    );
    struct Pending(Cell<usize>);
    impl EvaluationInputs for Pending {
        fn file(&self, _: &ResourcePath) -> InputRead {
            self.0.set(self.0.get() + 1);
            InputRead::Pending
        }
    }
    let pipeline = Pipeline::default();
    for source in [
        r#"#read_json(p"data")"#,
        r#"#self.load(p"data")"#,
        r#"#self.stop(p"data")"#,
        r#"#self.show(data_error_message(match read_json(p"data"){err(e)=>e,ok(d)=>panic("stop")}))"#,
    ] {
        let inputs = Pending(Cell::new(0));
        let analysis = pipeline
            .analyze("/project/page.not", source, &context)
            .unwrap();
        assert!(!analysis.diagnostics().is_empty(), "{source}");
        assert!(matches!(
            pipeline.evaluate_with_inputs(&analysis, &context, &inputs),
            EvaluationOutcome::Complete(_)
        ));
        assert_eq!(inputs.0.get(), 0);
    }
    let context = self::context(
        r#"fn show()->InlineContent{let value=comptime read_toml(p"data");[unused]}"#,
    );
    assert!(
        pipeline
            .check_module("view", &context)
            .iter()
            .any(|d| d.message.contains("not permitted during comptime"))
    );
}

#[test]
fn public_boundaries_reject_nested_views_and_inspection_only_gives_summaries() {
    let data = Value::Data(json(r#"{"private":"SECRET"}"#));
    for value in [
        data.clone(),
        Value::Array(vec![data.clone()].into()),
        Value::Ok(Box::new(data.clone())),
    ] {
        assert!(notist_html::components::encode_parameter(&value).is_err());
        assert!(
            !notist_html::components::value_json(&value)
                .to_string()
                .contains("SECRET")
        );
        let mut root = notist::Item::new(notist::Ctor::Doc, notist::TextRange::empty(0.into()));
        root.attrs.insert("hidden", value);
        let rendered = notist_html::Renderer::default().render_with_diagnostics(&root);
        assert!(
            rendered
                .diagnostics
                .iter()
                .any(|d| d.message.contains("Data"))
        );
        assert!(!rendered.html.contains("SECRET"));
    }
    let Err(DataFailure::Error(error)) = json("1").decode(&Type::String, DataLimits::default())
    else {
        panic!()
    };
    assert!(notist_html::components::encode_parameter(&Value::DataError(error)).is_err());
}

#[test]
fn projection_expansion_budget_is_incremental_and_cannot_be_swallowed() {
    let root = json("[1,2,3]");
    let mut target = Type::Int;
    for _ in 0..30 {
        target = Type::Option(Box::new(target));
    }
    let target = Type::Array(Some(Box::new(target)));
    assert!(matches!(
        root.decode(
            &target,
            DataLimits {
                nodes: 20,
                ..Default::default()
            }
        ),
        Err(DataFailure::Limit(_))
    ));
    let context = context(
        r#"fn show()->InlineContent{let d=data_expect<Data>(parse_json("[1,2,3]")); let s=match data_decode<Array<Int>>(d){ok(v)=>"value",err(e)=>"swallowed"};[#text(text:s)]}"#,
    );
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("/project/page.not", "#self.show()", &context)
        .unwrap();
    let limits = notist::Limits {
        data: DataLimits {
            nodes: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    let output = pipeline.evaluate_with_limits(&analysis, &context, limits);
    assert_eq!(output.diagnostics.len(), 1);
    assert!(!text(&output).contains("swallowed"));
}

#[test]
fn reexports_keep_implementation_identity_and_do_not_make_markup_data_calls_legal() {
    let mut files = MemoryResources::new("/project");
    files.insert("leaf.notc",r#"fn load(source:Path)->Data{data_expect<Data>(read_json(source))} fn stop(source:Path)->Never{let root=load(source);panic("stop")} fn body(source:Path)->InlineContent{let root=load(source);let name=data_expect<String>(data_require<String>(root,"name"));[#text(text:name)]}"#);
    files.insert(
        "middle.notc",
        "import \"leaf.notc\": load as value, stop as abort, body as display;",
    );
    files.insert("data.json", r#"{"name":"kept"}"#);
    let mut vault = Vault::new(files);
    for name in ["value", "abort"] {
        let source = format!("#import \"middle.notc\": {name} as alias;\n#alias(p\"data.json\")");
        let analysis = vault.analyze("bad.not", &source).unwrap();
        assert!(!analysis.diagnostics().is_empty());
        let evaluation = vault.evaluate("bad.not", &analysis).unwrap();
        assert!(evaluation.dependencies.is_empty());
        assert_eq!(vault.input_session().data_cache_stats().parses, 0);
    }
    let output = vault
        .render_html(
            "good.not",
            "#import \"middle.notc\": display as alias;\n#alias(p\"data.json\")",
            Default::default(),
        )
        .unwrap();
    assert!(output.evaluation.diagnostics.is_empty());
    assert!(output.rendered.html.contains("kept"));
}

#[test]
fn virtual_source_and_overlay_inputs_bind_data_to_the_same_immutable_text_view() {
    let mut files = MemoryResources::new("/project");
    files.insert("view.notc","fn show(source:Path)->InlineContent{let d=data_expect<Data>(read_json(source));let name=data_expect<String>(data_require<String>(d,\"name\"));[#text(text:name)]}");
    let source = r#"{"name":false}"#;
    let mut vault = Vault::new(files.clone());
    let _virtual = vault.analyze("virtual.not", source).unwrap();
    let output = vault
        .render_html(
            "page.not",
            "#import \"view.notc\" as view;\n#view.show(p\"virtual.not\")",
            Default::default(),
        )
        .unwrap();
    let error = &output.evaluation.diagnostics[0];
    assert_eq!(error.source.as_deref(), Some("/project/virtual.not"));
    assert_eq!(
        output.evaluation.sources[error.source.as_ref().unwrap()].as_ref(),
        source
    );
    files.insert("data.json", r#"{"name":"stale disk"}"#);
    let overrides =
        std::collections::BTreeMap::from([("data.json".into(), r#"{"name":"overlay"}"#.into())]);
    let resources = notist::OverlayResources::new(&files, &overrides);
    let mut vault = Vault::new(resources);
    let output = vault
        .render_html(
            "page.not",
            "#import \"view.notc\" as view;\n#view.show(p\"data.json\")",
            Default::default(),
        )
        .unwrap();
    assert!(output.evaluation.diagnostics.is_empty());
    assert!(output.rendered.html.contains("overlay"));
    assert!(!output.rendered.html.contains("stale disk"));
}

#[cfg(feature = "serde")]
#[test]
fn error_wire_drops_sources_and_special_native_implementations_are_reserved() {
    let d = json(r#"{"name":false,"private":"SECRET"}"#).bind_source(
        "/private/data.json".into(),
        notist::SourceLocation {
            source: "private.notc".into(),
            start: 0,
            end: 1,
        },
    );
    let Err(DataFailure::Error(error)) = d
        .require("name")
        .unwrap()
        .decode(&Type::String, DataLimits::default())
    else {
        panic!()
    };
    let encoded = serde_json::to_string(&Value::DataError(error.clone())).unwrap();
    assert!(!encoded.contains("SECRET"));
    assert!(!encoded.contains("private"));
    let Value::DataError(restored) = serde_json::from_str(&encoded).unwrap() else {
        panic!()
    };
    assert_eq!(restored, error);
    assert!(restored.diagnostic_source().is_none());
    let mut module = notist::DefinitionModule::new("fake");
    let mut definition = notist::builtins::registry()
        .function("notist", "read_json")
        .unwrap()
        .clone();
    definition.id = notist::FunctionId::new("fake", "read_json");
    module.functions.push(definition);
    assert!(
        notist_core::definitions::validate_module(&module)
            .iter()
            .any(|d| d.message.contains("reserved"))
    );
    let source = "fn read_json(source:Path)->Result<Data,DataError>; fn show()->InlineContent{let data=read_json(p\"file\");[bad]}";
    let ctx = context(source);
    assert!(
        Pipeline::default()
            .check_module("view", &ctx)
            .iter()
            .any(|d| d.message.contains("no implementation"))
    );
}

#[test]
fn tiny_views_cannot_hide_large_retained_documents_or_poison_failed_budget_checks() {
    let a = json(&format!(
        r#"{{"small":1,"padding":"{}"}}"#,
        "x".repeat(8192)
    ))
    .require("small")
    .unwrap();
    let b = json(&format!(
        r#"{{"small":2,"padding":"{}"}}"#,
        "y".repeat(8192)
    ))
    .require("small")
    .unwrap();
    assert_eq!(a.logical_size(), 8);
    let mut retention = notist_core::data::DataRetention::default();
    assert!(matches!(
        retention.observe(&Value::Data(a.clone()), 1024),
        Err(DataFailure::Limit(_))
    ));
    let array = Value::Array(vec![Value::Data(a), Value::Data(b)].into());
    for _ in 0..2 {
        assert!(matches!(
            retention.observe(&array, 20_000),
            Err(DataFailure::Limit(_))
        ));
    }
    let source = format!(r#"{{"small":1,"padding":"{}"}}"#, "x".repeat(8192));
    let module = format!(
        r##"fn small()->Data{{let d=data_expect<Data>(parse_json(r#"{source}"#));data_expect<Data>(data_require<Data>(d,"small"))}} fn show()->InlineContent{{let a=small();let b=small();[unreached]}}"##
    );
    let context = context(&module);
    let pipeline = Pipeline::default();
    let analysis = pipeline
        .analyze("/project/page.not", "#self.show()", &context)
        .unwrap();
    assert!(
        analysis.diagnostics().is_empty(),
        "{:?}",
        analysis.diagnostics()
    );
    let output = pipeline.evaluate_with_limits(
        &analysis,
        &context,
        notist::Limits {
            data: DataLimits {
                working_bytes: 32 * 1024,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    assert_eq!(output.diagnostics.len(), 1, "{:?}", output.diagnostics);
    assert!(output.diagnostics[0].message.contains("limit exceeded"));
    assert!(!text(&output).contains("unreached"));
}

#[test]
fn malformed_unicode_json_diagnostics_use_complete_character_boundaries() {
    let source = "{\"value\":😀}";
    let Err(DataFailure::Error(error)) =
        parse(DataFormat::Json, source.as_bytes(), DataLimits::default())
    else {
        panic!()
    };
    let error = error.bind_source(
        "input.json".into(),
        notist::SourceLocation {
            source: "read.notc".into(),
            start: 0,
            end: 1,
        },
    );
    let (_, text, span, _) = error.diagnostic_source().unwrap();
    assert_eq!(text.get(span), Some("😀"));
}

#[cfg(feature = "serde")]
#[test]
fn data_wire_is_bounded_selected_and_provenance_free_and_old_wire_stays_exact() {
    let d = json(r#"{"selected":[1,-0.0],"private":"SECRET"}"#)
        .require("selected")
        .unwrap();
    let encoded = serde_json::to_string(&Value::Data(d.clone())).unwrap();
    assert!(!encoded.contains("SECRET"));
    assert!(!encoded.contains("source"));
    let Value::Data(restored) = serde_json::from_str(&encoded).unwrap() else {
        panic!()
    };
    assert_eq!(restored, d);
    assert_eq!(restored.path(), "");
    assert_eq!(
        decode(&restored.require_index(1).unwrap(), Type::Float),
        Value::Float(-0.0)
    );
    let array = Value::Array(vec![Value::Str("s".into()), Value::Float(-0.0)].into());
    let encoded = serde_json::to_value(&array).unwrap();
    assert_eq!(
        encoded,
        serde_json::json!({"Array":[{"Str":"s"},{"Float":-0.0}]})
    );
    assert_eq!(serde_json::from_value::<Value>(encoded).unwrap(), array);
    assert!(serde_json::from_str::<Data>(r#"[{"Array":99999999}]"#).is_err());
    let deep = format!("[{}\"Null\"]", r#"{"Array":1},"#.repeat(65));
    assert!(serde_json::from_str::<Data>(&deep).is_err());
}
