//! Run with `cargo run --example prepared_preview`. A host prepares these
//! inputs asynchronously and transports them to a Worker before calling Vault.
use notist::{PreparedInputs, RenderOptions};
use std::{collections::BTreeMap, error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let inputs = PreparedInputs {
        root: "/notes".into(),
        config: None, // Select the nearest configuration for each document.
        files: BTreeMap::from([
            (
                PathBuf::from("Notist.toml"),
                b"[dependencies]\nwidgets = {path = 'packages/widgets'}".to_vec(),
            ),
            (
                PathBuf::from("packages/widgets/Notist.toml"),
                b"[package]\nname = 'widgets'".to_vec(),
            ),
            (
                PathBuf::from("packages/widgets/lib.notc"),
                include_bytes!("../packages/widgets/lib.notc").to_vec(),
            ),
            (
                PathBuf::from("packages/widgets/components/panel/index.js"),
                include_bytes!("../packages/widgets/components/panel/index.js").to_vec(),
            ),
            (
                PathBuf::from("packages/widgets/components/badge.js"),
                include_bytes!("../packages/widgets/components/badge.js").to_vec(),
            ),
        ]),
        // Publishing the directory preserves index.js's relative style import.
        module_urls: BTreeMap::from([
            (
                PathBuf::from("packages/widgets/components/panel/index.js"),
                "https://assets.example/widgets/components/panel/index.js".into(),
            ),
            (
                PathBuf::from("packages/widgets/components/badge.js"),
                "https://assets.example/widgets/components/badge.js".into(),
            ),
        ]),
    };
    // This round-trip stands in for a Worker message. Notist does no network IO.
    let message = serde_json::to_string(&inputs)?;
    let mut vault = serde_json::from_str::<PreparedInputs>(&message)?.into_vault();
    let output = vault.render_html(
        "preview.not",
        "#widgets::panel(title: \"Preview\")[#widgets::badge(\"nested\")]",
        RenderOptions::default(),
    )?;
    assert!(output.analysis.diagnostics().is_empty());
    assert!(output.transformed.diagnostics.is_empty());
    assert!(output.rendered.diagnostics.is_empty());
    println!("{}", output.rendered.html);
    for component in &output.rendered.used_components {
        println!("{}: {}", component.tag, component.module.url().unwrap());
    }
    println!("{} source mappings", output.rendered.source_map.len());
    Ok(())
}
