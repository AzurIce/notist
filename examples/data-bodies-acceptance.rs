//! Standalone full-size acceptance. See tools/data-bodies/README.md.
use notist::{
    Context, Ctor, DataCache, DataLimits, EvaluationInputs, EvaluationOutcome, InputError,
    InputRead, Item, Limits, Pipeline, ResourcePath,
};
use notist_core::data::{Data, DataFailure, DataFormat, ParsedDataCache};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

struct Inputs {
    files: BTreeMap<String, Arc<[u8]>>,
    cache: MeteredCache,
    requests: Cell<usize>,
}
struct MeteredCache {
    inner: DataCache,
    parsing: Cell<Duration>,
    projecting: Cell<Duration>,
}
impl ParsedDataCache for MeteredCache {
    fn parse(
        &self,
        format: DataFormat,
        bytes: Arc<[u8]>,
        limits: DataLimits,
    ) -> Result<Data, DataFailure> {
        let before = self.inner.stats().parses;
        let start = Instant::now();
        let result = self.inner.parse(format, bytes, limits);
        if self.inner.stats().parses != before {
            self.parsing.set(self.parsing.get() + start.elapsed());
        }
        result
    }
    fn decode(
        &self,
        data: &Data,
        target: &notist::Type,
        limits: DataLimits,
    ) -> Result<notist::Value, DataFailure> {
        let before = self.inner.stats().projections;
        let start = Instant::now();
        let result = self.inner.decode(data, target, limits);
        if self.inner.stats().projections != before {
            self.projecting.set(self.projecting.get() + start.elapsed());
        }
        result
    }
}
impl EvaluationInputs for Inputs {
    fn file(&self, path: &ResourcePath) -> InputRead {
        self.requests.set(self.requests.get() + 1);
        if self.requests.get() > 100_000 {
            return InputRead::Failed(InputError::RequestLimit);
        }
        if path.root() != "/validation" {
            return InputRead::Failed(InputError::AccessDenied);
        }
        self.files
            .get(path.path())
            .cloned()
            .map(InputRead::Ready)
            .unwrap_or(InputRead::Failed(InputError::NotFound))
    }
    fn data_cache(&self) -> Option<&dyn notist_core::data::ParsedDataCache> {
        Some(&self.cache)
    }
}
fn text(root: &Item) -> String {
    root.descendants()
        .filter(|n| matches!(n.ctor(), Ctor::Text | Ctor::RawInline))
        .filter_map(|n| match n.field("text") {
            Some(notist::Value::Str(s)) => Some(s.as_str()),
            _ => None,
        })
        .collect()
}
fn peak_rss_kib() -> Option<usize> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|l| l.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        3,
        "usage: data-bodies-acceptance FIXTURE OUTPUT"
    );
    let fixture = Path::new(&args[1]);
    let output = PathBuf::from(&args[2]);
    assert!(!output.exists(), "use a new output directory");
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(fixture.join("manifest.json")).unwrap()).unwrap();
    let mut files = BTreeMap::new();
    files.insert(
        "items.json".into(),
        Arc::from(std::fs::read(fixture.join("items.json")).unwrap()),
    );
    for file in manifest["catalogs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
    {
        files.insert(
            file.into(),
            Arc::from(std::fs::read(fixture.join(file)).unwrap()),
        );
    }
    for name in manifest["ranim"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
    {
        let file = format!("ranim/{name}.json");
        files.insert(
            file.clone(),
            Arc::from(std::fs::read(fixture.join(file)).unwrap()),
        );
    }
    let inputs = Inputs {
        files,
        cache: MeteredCache {
            inner: DataCache::new(1024 * 1024 * 1024, 4096),
            parsing: Cell::new(Duration::ZERO),
            projecting: Cell::new(Duration::ZERO),
        },
        requests: Cell::new(0),
    };
    let source_bytes: usize = inputs.files.values().map(|v| v.len()).sum();
    let pipeline = Pipeline::default();
    let module = pipeline
        .compile_module_with_root(
            "page.not",
            "view",
            "/validation/view.notc",
            include_str!("../tools/data-bodies/view.notc"),
            "/validation",
        )
        .unwrap();
    let context = Context::default()
        .with_resource_root("/validation")
        .with_module(module)
        .unwrap();
    let limits = Limits {
        value_bytes: 32 * 1024 * 1024,
        data: DataLimits {
            nodes: 1_000_000,
            value_bytes: 32 * 1024 * 1024,
            working_bytes: 1024 * 1024 * 1024,
            ..DataLimits::default()
        },
        ..Limits::default()
    };
    // A full logical store must fail under defaults, even when the byte source fits.
    let bytes = &inputs.files["items.json"];
    assert!(
        notist_core::data::parse(notist::DataFormat::Json, bytes, DataLimits::default()).is_err()
    );
    let reference: Value = serde_json::from_slice(bytes).unwrap();
    std::fs::create_dir_all(&output).unwrap();
    let start = Instant::now();
    let mut analysis_time = Duration::ZERO;
    let mut evaluation_time = Duration::ZERO;
    let mut rendering_time = Duration::ZERO;
    let mut write_time = Duration::ZERO;
    let mut specializations = BTreeSet::new();
    let mut checked_max = 0;
    let mut html_bytes = 0usize;
    let mut pages = 0usize;
    let mut nodes = 0usize;
    let mut run = |call: String, path: String, verify: &dyn Fn(&Item, &str)| {
        let now = Instant::now();
        let analysis = pipeline
            .analyze("/validation/page.not", &call, &context)
            .unwrap();
        assert!(
            analysis.diagnostics().is_empty(),
            "{path}: {:?}",
            analysis.diagnostics()
        );
        checked_max = checked_max.max(analysis.checked_instance_count());
        analysis_time += now.elapsed();
        let now = Instant::now();
        let EvaluationOutcome::Complete(result) =
            pipeline.evaluate_with_inputs_and_limits(&analysis, &context, &inputs, limits)
        else {
            panic!("all inputs are immutable and prepared");
        };
        assert!(
            result.diagnostics.is_empty(),
            "{path}: {:?}",
            result.diagnostics
        );
        evaluation_time += now.elapsed();
        for node in result.root.descendants() {
            assert!(node.as_expr().is_none());
            if let Some(instance) = node.instance() {
                specializations.insert(instance.key(result.sources.definitions()).unwrap());
            }
            nodes += 1;
        }
        let now = Instant::now();
        let rendered = notist_html::Renderer::new().render_with_diagnostics(&result.root);
        assert!(
            rendered.diagnostics.is_empty(),
            "{path}: {:?}",
            rendered.diagnostics
        );
        rendering_time += now.elapsed();
        verify(&result.root, &rendered.html);
        html_bytes += rendered.html.len();
        let now = Instant::now();
        let destination = output.join(path);
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(destination, rendered.html).unwrap();
        write_time += now.elapsed();
        pages += 1;
    };
    for key in manifest["ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
    {
        let record = &reference[key];
        run(
            format!("#self.detail(p\"./items.json\", {key:?})"),
            format!("items/{key}.html"),
            &|root, html| {
                let rows: Vec<_> = root
                    .descendants()
                    .filter(|n| n.ctor() == Ctor::TableRow)
                    .collect();
                assert_eq!(rows.len(), 13);
                for row in rows {
                    let name = text(&row.children()[0]);
                    let actual = text(&row.children()[1]);
                    let expected = match &record[&name] {
                        Value::String(s) => s.clone(),
                        Value::Bool(b) => if *b { "是" } else { "否" }.into(),
                        Value::Number(n) => n.to_string(),
                        other => panic!("unexpected field {name}: {other}"),
                    };
                    assert_eq!(actual, expected, "item {key} field {name}");
                }
                assert!(html.contains(&notist_html::escape_text(record["name"].as_str().unwrap())));
            },
        );
    }
    let detail_elapsed = start.elapsed();
    for file in manifest["catalogs"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
    {
        let records: Value = serde_json::from_slice(&inputs.files[file]).unwrap();
        run(
            format!("#self.catalog(p\"./{file}\")"),
            file.replace(".json", ".html"),
            &|root, _| {
                let rows: Vec<_> = root
                    .descendants()
                    .filter(|n| n.ctor() == Ctor::TableRow)
                    .collect();
                let expected = records.as_array().unwrap();
                assert_eq!(rows.len(), expected.len());
                for (row, record) in rows.into_iter().zip(expected) {
                    assert_eq!(text(&row.children()[0]), record["id"].to_string());
                    assert_eq!(text(&row.children()[1]), record["name"].as_str().unwrap());
                    assert_eq!(text(&row.children()[2]), record["category"].to_string());
                }
            },
        );
    }
    for name in manifest["ranim"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
    {
        let file = format!("ranim/{name}.json");
        let record: Value = serde_json::from_slice(&inputs.files[&file]).unwrap();
        run(
            format!("#self.example(p\"./{file}\")"),
            format!("ranim/{name}.html"),
            &|root, html| {
                let raw: Vec<_> = root
                    .descendants()
                    .filter(|n| {
                        n.ctor() == Ctor::RawInline && n.level == notist::builtins::Level::Block
                    })
                    .collect();
                assert_eq!(raw.len(), 2);
                for (raw, source) in raw.into_iter().zip(record["sources"].as_array().unwrap()) {
                    assert_eq!(
                        raw.field("text"),
                        Some(&notist::Value::Str(source["text"].as_str().unwrap().into()))
                    );
                }
                let links: Vec<_> = root
                    .descendants()
                    .filter(|n| n.ctor() == Ctor::Link)
                    .collect();
                assert_eq!(
                    links.len(),
                    record["preview_imgs"].as_array().unwrap().len()
                        + record["output_files"].as_array().unwrap().len()
                );
                for output in record["output_files"].as_array().unwrap() {
                    assert!(html.contains(output["kind"].as_str().unwrap()));
                }
            },
        );
    }
    // Real-data functional parity, not an additional production page model.
    let markdown = Context::default()
        .with_resource_root("/validation")
        .with_module(
            pipeline
                .compile_module_with_root(
                    "page.md",
                    "view",
                    "/validation/view.notc",
                    include_str!("../tools/data-bodies/view.notc"),
                    "/validation",
                )
                .unwrap(),
        )
        .unwrap();
    let media_example = manifest["ranim"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|name| {
            let record: Value =
                serde_json::from_slice(&inputs.files[&format!("ranim/{name}.json")]).unwrap();
            !record["output_files"].as_array().unwrap().is_empty()
        })
        .expect("real fixture includes published outputs");
    let functional_calls = [
        format!("#self.detail(p\"./items.json\", {})", manifest["ids"][0]),
        format!(
            "#self.catalog(p\"./{}\")",
            manifest["catalogs"][0].as_str().unwrap()
        ),
        format!("#self.example(p\"./ranim/{media_example}.json\")"),
    ];
    for call in &functional_calls {
        let mut outputs = Vec::new();
        for (path, context) in [
            ("/validation/page.not", &context),
            ("/validation/page.md", &markdown),
        ] {
            let analysis = pipeline.analyze(path, call, context).unwrap();
            assert!(
                analysis.diagnostics().is_empty(),
                "{path}: {:?}",
                analysis.diagnostics()
            );
            let EvaluationOutcome::Complete(result) =
                pipeline.evaluate_with_inputs_and_limits(&analysis, context, &inputs, limits)
            else {
                panic!("prepared snapshot");
            };
            assert!(
                result.diagnostics.is_empty(),
                "{path}: {:?}",
                result.diagnostics
            );
            let rendered = notist_html::Renderer::new().render_with_diagnostics(&result.root);
            assert!(rendered.diagnostics.is_empty());
            outputs.push(rendered.html);
        }
        assert_eq!(outputs[0], outputs[1], "real-data frontend parity: {call}");
    }
    let cache = inputs.cache.inner.stats();
    let full_store = inputs
        .cache
        .inner
        .parse(
            DataFormat::Json,
            inputs.files["items.json"].clone(),
            limits.data,
        )
        .unwrap();
    assert_eq!(
        inputs.cache.inner.stats().parses,
        cache.parses,
        "logical size inspection must reuse the parsed store"
    );
    assert_eq!(
        cache.parses,
        inputs.files.len(),
        "each immutable file is parsed once"
    );
    assert!(checked_max < 100, "no per-record comptime specialization");
    let report = json!({
        "items": manifest["items"], "catalogs": manifest["catalogs"].as_array().unwrap().len(),
        "ranim": manifest["ranim"].as_array().unwrap().len(), "pages": pages, "html_bytes": html_bytes,
        "nodes": nodes, "csv_sha256": manifest["csv_sha256"], "input_bytes": source_bytes,
        "items_source_bytes": inputs.files["items.json"].len(), "items_logical_bytes": full_store.logical_size(),
        "items_nodes": full_store.node_count(),
        "elapsed_seconds": start.elapsed().as_secs_f64(), "details_seconds": detail_elapsed.as_secs_f64(),
        "analysis_seconds": analysis_time.as_secs_f64(), "evaluation_seconds": evaluation_time.as_secs_f64(),
        "rendering_seconds": rendering_time.as_secs_f64(), "write_seconds": write_time.as_secs_f64(),
        "cold_parse_seconds_in_evaluation": inputs.cache.parsing.get().as_secs_f64(),
        "cold_projection_seconds_in_evaluation": inputs.cache.projecting.get().as_secs_f64(),
        "peak_rss_kib": peak_rss_kib(), "read_requests": inputs.requests.get(),
        "cross_frontend_cases": functional_calls.len(),
        "parses": cache.parses, "projections": cache.projections, "cache_bytes": cache.bytes,
        "cache_entries": cache.entries, "max_checked_instances_per_page": checked_max,
        "mounted_specializations": specializations.len(),
        "limits": {"data_source_bytes": limits.data.source_bytes, "data_logical_bytes": limits.data.value_bytes,
            "data_nodes": limits.data.nodes, "data_working_bytes": limits.data.working_bytes,
            "value_bytes": limits.value_bytes, "cache_bytes": 1024 * 1024 * 1024, "cache_entries": 4096,
            "host_read_requests": 100000},
        "scope": manifest["scope"],
    });
    std::fs::write(
        output.join("report.json"),
        serde_json::to_string_pretty(&report).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}
