//! cargo run -p notist-cli --release --features profiling --example module_cache_profile -- synthetic 1000 3
//! Modes: synthetic [pages] [rounds], site ROOT [rounds], book ROOT [rounds].
//! Each round creates a fresh Vault. Site/book assemble full outputs, without publishing.
use notist::{FsResources, MemoryResources, Vault, profiling};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Instant,
};

fn report(mode: &str, round: usize, started: Instant, pages: usize, files: usize, bytes: usize) {
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
    println!(
        "{}",
        json!({"mode": mode, "round": round, "wall_ms": started.elapsed().as_secs_f64() * 1e3,
        "pages": pages, "files": files, "bytes": bytes, "stages": stages})
    );
}

fn synthetic(pages: usize, rounds: usize) {
    let mut resources = MemoryResources::new("/profile");
    let leaf = (0..24)
        .map(|i| format!("fn value{i}() -> String {{ \"text{i}\" }}\n"))
        .collect::<String>();
    resources.insert("leaf.notc", leaf.into_bytes());
    for i in 0..8 {
        resources.insert(format!("view{i}.notc"), format!(
            "import \"leaf.notc\": *; fn show() -> InlineContent {{ [#text(text: value{i}())] }}"
        ).into_bytes());
    }
    for round in 0..rounds {
        profiling::take();
        let started = Instant::now();
        let mut vault = Vault::new(resources.clone());
        for page in 0..pages {
            let source = format!("#import \"view{}.notc\" as view;\n#view.show()", page % 8);
            let analysis = vault.analyze(format!("page{page}.not"), &source).unwrap();
            assert!(
                analysis.diagnostics().is_empty(),
                "{:?}",
                analysis.diagnostics()
            );
            let output = vault
                .evaluate(format!("page{page}.not"), &analysis)
                .unwrap();
            assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
            std::hint::black_box(output);
        }
        report("synthetic", round, started, pages, 0, 0);
    }
}

fn project(mode: &str, root: &Path, rounds: usize, output: Option<&Path>) {
    let root = root.canonicalize().unwrap();
    let text = std::fs::read_to_string(root.join("Notist.toml")).unwrap();
    let manifest: toml::Value = toml::from_str(&text).unwrap();
    let mut table = manifest[mode][0].as_table().unwrap().clone();
    table.remove("name");
    table
        .entry("output")
        .or_insert_with(|| toml::Value::String("profile-output".into()));
    for round in 0..rounds {
        profiling::take();
        let started = Instant::now();
        // Explicit, identical workload policy for all compared implementations.
        // The default 16,384 source/data requests cannot finish a 51k-page site.
        let mut vault = Vault::new(FsResources::new(&root))
            .with_input_limits(notist::InputLimits {
                requests: 1_000_000,
                ..Default::default()
            })
            .with_config(root.join("Notist.toml"));
        let (pages, files) = if mode == "site" {
            let config: notist_ssg::SiteConfig =
                toml::Value::Table(table.clone()).try_into().unwrap();
            let output = notist_ssg::Builder::default()
                .build(&mut vault, &config)
                .unwrap();
            (output.pages.len(), output.files)
        } else {
            let config: notist_book::BookConfig =
                toml::Value::Table(table.clone()).try_into().unwrap();
            let output = notist_book::build(&mut vault, &config).unwrap();
            (output.pages.len(), output.files)
        };
        report(
            mode,
            round,
            started,
            pages,
            files.len(),
            files.values().map(Vec::len).sum(),
        );
        if round + 1 == rounds
            && let Some(output) = output
        {
            for (path, bytes) in &files {
                let path = output.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, bytes).unwrap();
            }
        }
        std::hint::black_box(files);
    }
}

fn main() {
    let args: Vec<_> = std::env::args().collect();
    profiling::set_cache_mode(match args.get(5).map(String::as_str).unwrap_or("both") {
        "baseline" => profiling::CacheMode::Baseline,
        "compile" => profiling::CacheMode::CompileOnly,
        "both" => profiling::CacheMode::Both,
        other => panic!("unknown cache mode {other}"),
    });
    let mode = args.get(1).map(String::as_str).unwrap_or("synthetic");
    let rounds = args.get(3).map(|n| n.parse().unwrap()).unwrap_or(3);
    if mode == "synthetic" {
        synthetic(
            args.get(2).map(|n| n.parse().unwrap()).unwrap_or(1000),
            rounds,
        );
    } else {
        assert!(matches!(mode, "site" | "book"));
        project(
            mode,
            &PathBuf::from(args.get(2).expect("project root")),
            rounds,
            args.get(4).map(Path::new),
        );
    }
}
