//! Complete, module-rich site benchmark. See tools/MODULE_SITE_PROFILE.md.
//! Run one cache mode per fresh process. Fixture edits, validation copies, and
//! diagnostic serialization are outside the assembly timer.
use notist::{InputLimits, MemoryResources, Vault, profiling};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path, time::Instant};

fn load(directory: &Path, root: &Path, resources: &mut MemoryResources) {
    let mut entries: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            load(&path, root, resources);
        } else {
            resources.insert(
                path.strip_prefix(root).unwrap(),
                std::fs::read(&path).unwrap(),
            );
        }
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(args.len(), 4, "module_site_profile FIXTURE MODE OUTPUT");
    let fixture = Path::new(&args[1]);
    let output = Path::new(&args[3]);
    assert!(!output.exists(), "use a new output directory");
    let spec: Value =
        serde_json::from_slice(&std::fs::read(fixture.join("fixture.json")).unwrap()).unwrap();
    profiling::set_cache_mode(match args[2].as_str() {
        "off" => profiling::CacheMode::Baseline,
        "compile" => profiling::CacheMode::CompileOnly,
        "both" => profiling::CacheMode::Both,
        other => panic!("unknown cache mode {other}"),
    });
    let mut base = MemoryResources::new("/module-site");
    load(&fixture.join("site"), &fixture.join("site"), &mut base);
    let config: notist_ssg::SiteConfig = serde_json::from_value(spec["config"].clone()).unwrap();
    for step in spec["scenarios"].as_array().unwrap() {
        let name = step["name"].as_str().unwrap();
        let mut resources = base.clone();
        for (path, text) in step["overrides"].as_object().unwrap() {
            resources.insert(path, text.as_str().unwrap().as_bytes().to_vec());
        }
        profiling::take();
        let started = Instant::now();
        // Match ordinary host batch maintenance, with one explicit eviction case.
        notist::evict_caches(if name == "evicted" { 0 } else { 10 });
        let mut vault = Vault::new(resources)
            .with_input_limits(InputLimits {
                requests: 1_000_000,
                ..Default::default()
            })
            .with_config("Notist.toml");
        let result = notist_ssg::Builder::default().build(&mut vault, &config);
        drop(vault);
        let assembly_ms = started.elapsed().as_secs_f64() * 1000.0;
        let stats = profiling::take();
        let mut stages = BTreeMap::new();
        for (name, m) in [
            ("compile", stats.compile),
            ("compile_work", stats.compile_work),
            ("bind", stats.bind),
            ("bind_work", stats.bind_work),
            ("link", stats.link),
            ("install", stats.install),
        ] {
            stages.insert(name, json!({"calls": m.calls, "ms": m.nanos as f64 / 1e6}));
        }
        let destination = output.join(name);
        std::fs::create_dir_all(&destination).unwrap();
        let (pages, files, bytes, errors) = match result {
            Ok(artifact) => {
                assert!(
                    !step["error"].as_bool().unwrap(),
                    "expected an error in {name}"
                );
                assert!(artifact.warnings.is_empty(), "{:?}", artifact.warnings);
                let counts = (
                    artifact.pages.len(),
                    artifact.files.len(),
                    artifact.files.values().map(Vec::len).sum::<usize>(),
                    0,
                );
                assert_eq!(
                    artifact.pages.len(),
                    spec["page_count"].as_u64().unwrap() as usize
                );
                for (path, bytes) in &artifact.files {
                    let path = destination.join(path);
                    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                    std::fs::write(path, bytes).unwrap();
                }
                counts
            }
            Err(notist_ssg::Error::Diagnostics(errors)) => {
                assert!(step["error"].as_bool().unwrap(), "{name}: {errors:#?}");
                assert!(
                    errors.iter().any(|e| e
                        .diagnostic
                        .message
                        .contains("has no public member `stamp`")),
                    "{errors:#?}"
                );
                let diagnostics: Vec<_> = errors.iter().map(|e| json!({
                    "path": e.path, "source": e.source,
                    "span": [u32::from(e.diagnostic.span.start()), u32::from(e.diagnostic.span.end())],
                    "phase": e.diagnostic.phase.to_string(), "message": e.diagnostic.message,
                })).collect();
                std::fs::write(
                    destination.join("errors.json"),
                    serde_json::to_vec_pretty(&diagnostics).unwrap(),
                )
                .unwrap();
                (0, 0, 0, errors.len())
            }
            Err(error) => panic!("{name}: {error}"),
        };
        println!(
            "{}",
            json!({"scenario": name, "mode": args[2], "assembly_ms": assembly_ms,
            "pages": pages, "files": files, "bytes": bytes, "errors": errors,
            "stages": stages, "output": destination})
        );
    }
}
