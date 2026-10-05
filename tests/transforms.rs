use notist::{Ctor, FunctionId, MemoryResources, PreparedInputs, RenderOptions, Vault, VaultError};
use std::{collections::BTreeMap, path::Path};

const CONFIG: &str = "[dependencies]\nkatex = { path = '../packages/katex' }\n\n[[transforms]]\nkind = 'replace'\nfrom = 'notist::math'\nto = 'katex::math'\n";
const DECLARATION: &str = "fn math(text: String, block?: Bool) -> Content<block>;";

fn resources(config: &str, declaration: &str, component: bool) -> MemoryResources {
    let mut resources = MemoryResources::new("/repo/docs");
    resources.insert("Notist.toml", config.as_bytes().to_vec());
    resources.insert(
        "../packages/katex/lib.notc",
        declaration.as_bytes().to_vec(),
    );
    resources.insert(
        "../packages/katex/Notist.toml",
        b"[package]\nname = 'katex'".to_vec(),
    );
    if component {
        resources.insert(
            "../packages/katex/components/math.js",
            b"export default class extends HTMLElement {}".to_vec(),
        );
    }
    resources
}

#[test]
fn configuration_parses_ordered_rules_and_rejects_unsupported_or_malformed_entries() {
    let source = format!(
        "{CONFIG}\n[[transforms]]\nkind = 'replace'\nfrom = 'katex::math'\nto = 'notist::math'\n"
    );
    let config = notist::environment::parse_config(&source).unwrap();
    assert_eq!(config.dependencies[0].path, Path::new("../packages/katex"));
    assert!(config.dependencies[0].transforms);
    assert_eq!(config.transforms.len(), 2);
    assert_eq!(config.transforms[0].from, FunctionId::new("notist", "math"));
    assert_eq!(config.transforms[1].from, FunctionId::new("katex", "math"));
    assert!(config.transforms[0].span.end() <= config.transforms[1].span.start());
    assert!(
        notist::environment::parse_config("")
            .unwrap()
            .transforms
            .is_empty()
    );
    for invalid in [
        CONFIG.replace("notist::math", "math"),
        CONFIG.replace("notist::math", "notist::nested::math"),
        CONFIG.replace("katex::math", "katex::"),
        CONFIG.replace("kind = 'replace'", "kind = 'command'"),
        CONFIG.replace("kind = 'replace'", ""),
        CONFIG.replace("kind = 'replace'", "kind = 1"),
        CONFIG.replace("from = 'notist::math'", "from = 1"),
        CONFIG.replace("to = 'katex::math'", ""),
        CONFIG.replace("path = '../packages/katex'", "path = 1"),
        CONFIG.replace("path = '../packages/katex'", "unexpected = true"),
        CONFIG.replace("path = '../packages/katex'", "path = ''"),
        CONFIG.replace(
            "path = '../packages/katex'",
            "path = '../packages/katex', transforms = 'false'",
        ),
        CONFIG.replace(
            "path = '../packages/katex'",
            "path = '../packages/katex', transforms = 0",
        ),
        CONFIG.replace("katex =", "notist ="),
        "dependencies = []".to_owned(),
        "transforms = false".to_owned(),
    ] {
        let errors = notist::environment::parse_config(&invalid).unwrap_err();
        assert!(
            errors.iter().all(|error| !error.span.is_empty()),
            "{errors:?}"
        );
    }
    // An incomplete TOML header can report an empty span at end of input.
    assert!(notist::environment::parse_config("[dependencies").is_err());
}

#[test]
fn dependency_transform_switches_parse_and_are_exposed_to_preview() {
    let source = "[dependencies]\n\
                  default = {path = 'default'}\n\
                  disabled = {path = 'disabled', transforms = false}\n\
                  enabled = {path = 'enabled', transforms = true}\n\
                  [dev-dependencies]\n\
                  demo = {path = 'demo', transforms = false}";
    let config = notist::environment::parse_config(source).unwrap();
    assert_eq!(
        config
            .dependencies
            .iter()
            .map(|dependency| dependency.transforms)
            .collect::<Vec<_>>(),
        [true, false, true]
    );
    assert!(!config.dev_dependencies[0].transforms);
    let description: serde_json::Value =
        serde_json::from_str(&notist::preview::configuration_json(source)).unwrap();
    assert_eq!(description["dependencies"][0]["transforms"], true);
    assert_eq!(description["dependencies"][1]["transforms"], false);
    assert_eq!(description["dependencies"][2]["transforms"], true);
    assert_eq!(description["dev_dependencies"][0]["transforms"], false);
}

#[test]
fn unknown_configuration_fields_preserve_dependencies_and_transforms() {
    let source = "\
future_option = true

[future_section]
options = { nested = [1, 2] }

[[future_rules]]
kind = 'future'

[dependencies]
katex = { path = '../packages/katex', future_option = true, options = { nested = [1, 2] } }

[[transforms]]
kind = 'replace'
from = 'notist::math'
to = 'katex::math'
future_option = true
options = { nested = [1, 2] }
";
    let config = notist::environment::parse_config(source).unwrap();
    assert_eq!(config.dependencies.len(), 1);
    assert_eq!(config.dependencies[0].name, "katex");
    assert_eq!(config.dependencies[0].path, Path::new("../packages/katex"));
    assert_eq!(config.transforms.len(), 1);
    assert_eq!(config.transforms[0].from, FunctionId::new("notist", "math"));
    assert_eq!(config.transforms[0].to, FunctionId::new("katex", "math"));

    let description: serde_json::Value =
        serde_json::from_str(&notist::preview::configuration_json(source)).unwrap();
    assert_eq!(description["dependencies"][0]["name"], "katex");
    assert_eq!(description["transforms"][0]["kind"], "replace");

    let mut vault = Vault::new(resources(source, DECLARATION, true));
    let output = vault
        .render_html("doc.not", "$x$", RenderOptions::default())
        .unwrap();
    assert!(output.rendered.html.contains("<katex-math "));
}

#[test]
fn vault_output_transforms_once_and_keeps_source_analysis_and_maps() {
    let mut vault = Vault::new(resources(CONFIG, DECLARATION, true));
    for (path, source) in [("doc.not", "$x$ #math(\"y\")"), ("doc.md", "- $x$\n\n$y$")] {
        let output = vault
            .render_html(path, source, RenderOptions::default())
            .unwrap();
        assert!(output.analysis.diagnostics().is_empty());
        assert!(output.transformed.diagnostics.is_empty());
        assert!(output.rendered.diagnostics.is_empty());
        assert_eq!(
            output
                .analysis
                .root()
                .descendants()
                .filter(|node| node.ctor == Ctor::Math)
                .count(),
            2
        );
        assert_eq!(
            output
                .transformed
                .root
                .descendants()
                .filter(|node| node.function_id() == Some(FunctionId::new("katex", "math")))
                .count(),
            2
        );
        assert_eq!(output.rendered.html.matches("<katex-math ").count(), 2);
        assert_eq!(output.rendered.used_components.len(), 1);
        assert_eq!(
            output.rendered.used_components[0].module.resource(),
            Some(Path::new("/repo/packages/katex/components/math.js"))
        );
        assert!(!output.rendered.html.contains("notist-math"));
        let math_spans: Vec<_> = output
            .analysis
            .root()
            .descendants()
            .filter(|node| node.ctor == Ctor::Math)
            .map(|node| node.span)
            .collect();
        for span in math_spans {
            assert!(
                output
                    .rendered
                    .source_map
                    .iter()
                    .any(|mapping| mapping.range.start == usize::from(span.start())
                        && mapping.range.end == usize::from(span.end()))
            );
        }
        let inspected = vault.inspect(path, source).unwrap().0;
        let debug = vault
            .render_output(path, inspected.root(), RenderOptions::default())
            .unwrap();
        assert_eq!(debug.transformed, output.transformed);
        assert_eq!(debug.rendered.html, output.rendered.html);
        assert_eq!(vault.analyze(path, source).unwrap(), output.analysis);
    }
    // Rules can return to a native target without a component dependency.
    let roundtrip = format!(
        "{CONFIG}\n[[transforms]]\nkind = 'replace'\nfrom = 'katex::math'\nto = 'notist::math'\n"
    );
    let mut vault = Vault::new(resources(&roundtrip, DECLARATION, true));
    let output = vault
        .render_html("doc.not", "$x$", RenderOptions::default())
        .unwrap();
    assert_eq!(output.transformed.root, *output.analysis.root());
    assert!(output.rendered.used_components.is_empty());
    // This list is deliberately not idempotent: applying it twice changes
    // math to done. Both output entry points must execute it exactly once.
    let config = "[dependencies]\nkatex = {path = '../packages/katex'}\n\n[[transforms]]\nkind = 'replace'\nfrom = 'katex::math'\nto = 'katex::done'\n\n[[transforms]]\nkind = 'replace'\nfrom = 'notist::math'\nto = 'katex::math'\n";
    let declaration =
        format!("{DECLARATION} fn done(text: String, block?: Bool) -> Content<block>;");
    let mut vault = Vault::new(resources(config, &declaration, true));
    let output = vault
        .render_html("doc.not", "$x$", RenderOptions::default())
        .unwrap();
    assert_eq!(
        output.rendered.used_components[0].id,
        FunctionId::new("katex", "math")
    );
    let inspected = vault.inspect("doc.not", "$x$").unwrap().0;
    let debug = vault
        .render_output("doc.not", inspected.root(), RenderOptions::default())
        .unwrap();
    assert_eq!(debug.transformed, output.transformed);
}

#[test]
fn environment_failures_retain_config_source_and_rule_span() {
    for (config, declaration, reason) in [
        (
            CONFIG.replace("katex::math", "missing::math"),
            DECLARATION,
            "unknown",
        ),
        (
            CONFIG.to_owned(),
            "fn math(source: String, block?: Bool) -> Content<block>;",
            "parameter",
        ),
        (
            CONFIG.to_owned(),
            "fn math(text: String) -> Content;",
            "levels",
        ),
    ] {
        let mut vault = Vault::new(resources(&config, declaration, false));
        let VaultError::Environment(errors) = vault.analyze("doc.not", "$x$").unwrap_err() else {
            panic!("expected configuration error")
        };
        assert!(
            errors
                .iter()
                .any(|error| error.diagnostic.message.contains(reason)),
            "{errors:?}"
        );
        for error in errors {
            assert_eq!(error.path, Path::new("/repo/docs/Notist.toml"));
            assert_eq!(error.source, config);
            assert!(
                config[usize::from(error.diagnostic.span.start())
                    ..usize::from(error.diagnostic.span.end())]
                    .contains("from")
            );
        }
    }
}

#[test]
fn missing_components_do_not_invalidate_transforms_and_no_config_keeps_defaults() {
    let mut vault = Vault::new(resources(CONFIG, DECLARATION, false));
    let analysis = vault.analyze("doc.not", "$x$").unwrap();
    assert!(analysis.diagnostics().is_empty());
    let transformed = vault.transform("doc.not", analysis.root()).unwrap();
    assert!(transformed.diagnostics.is_empty());
    let rendered = vault
        .render_html("doc.not", "$x$", RenderOptions::default())
        .unwrap();
    assert!(
        rendered.rendered.diagnostics[0]
            .message
            .contains("no HTML implementation")
    );
    let mut empty = Vault::new(MemoryResources::new("/plain"));
    let output = empty
        .render_html("doc.not", "$x$", RenderOptions::default())
        .unwrap();
    assert_eq!(output.transformed.root, *output.analysis.root());
    assert!(output.rendered.html.contains("notist-math"));
    assert!(output.rendered.used_components.is_empty());
}

#[test]
fn native_preview_worker_and_debug_paths_share_transforms_and_diagnostic_origins() {
    let source = "$x$ #math(false)";
    let config = CONFIG.replace("../packages/katex", "./packages/katex");
    let inputs = PreparedInputs {
        root: "/preview".into(),
        config: Some("Notist.toml".into()),
        files: BTreeMap::from([
            ("Notist.toml".into(), config.as_bytes().to_vec()),
            (
                "packages/katex/Notist.toml".into(),
                b"[package]\nname = 'katex'".to_vec(),
            ),
            (
                "packages/katex/lib.notc".into(),
                DECLARATION.as_bytes().to_vec(),
            ),
            ("packages/katex/components/math.js".into(), Vec::new()),
        ]),
        module_urls: BTreeMap::from([(
            "packages/katex/components/math.js".into(),
            "https://host.test/math.js".into(),
        )]),
    };
    let message = serde_json::to_string(&inputs).unwrap();
    let native = inputs
        .clone()
        .into_vault()
        .render_html("doc.not", source, RenderOptions::default())
        .unwrap();
    let render: serde_json::Value = serde_json::from_str(&notist::preview::render_prepared(
        "doc.not", source, &message,
    ))
    .unwrap();
    let debug: serde_json::Value = serde_json::from_str(&notist::preview::analyze_prepared(
        "doc.not", source, &message,
    ))
    .unwrap();
    assert_eq!(render["html"], native.rendered.html);
    for key in ["html", "diagnostics", "used_components", "source_map"] {
        assert_eq!(render[key], debug[key], "{key}");
    }
    assert!(
        render["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["origin"] == "transform" && error["path"] == "doc.not")
    );
    assert!(
        render["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["origin"] == "analysis")
    );
    assert_eq!(render["used_components"][0]["package"], "katex");
    assert!(debug["transformed"].to_string().contains("katex::math"));
    assert!(debug["core"].to_string().contains("notist::math"));
}
