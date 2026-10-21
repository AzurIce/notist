//! Run with `cargo run --example prepared_preview`. A host prepares these
//! inputs asynchronously and transports them to a Worker before calling Vault.
use notist::{PreparedInputs, RenderOptions};
use std::{collections::BTreeMap, error::Error, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let inputs = PreparedInputs {
        root: "/notes".into(),
        config: None, // No output configuration is needed.
        files: BTreeMap::from([
            (
                PathBuf::from("Notist.toml"),
                b"".to_vec(),
            ),
            (
                PathBuf::from("lib.notc"),
                b"fn panel(title: String = \"Panel\", expanded: Bool = true)[children: BlockContent] -> BlockContent; fn badge(label: String, count: Int = 1) -> InlineContent;".to_vec(),
            ),
            (
                PathBuf::from("panel/index.js"),
                include_bytes!("../docs/components/panel/index.js").to_vec(),
            ),
            (
                PathBuf::from("panel/style.js"),
                include_bytes!("../docs/components/panel/style.js").to_vec(),
            ),
            (
                PathBuf::from("badge.js"),
                include_bytes!("../docs/components/badge.js").to_vec(),
            ),
        ]),
        // Publishing the directory preserves index.js's relative style import.
        module_urls: BTreeMap::from([
            (
                PathBuf::from("panel/index.js"),
                "https://assets.example/notist-doc/panel/index.js".into(),
            ),
            (
                PathBuf::from("badge.js"),
                "https://assets.example/notist-doc/badge.js".into(),
            ),
        ]),
    };
    // This round-trip stands in for a Worker message. Notist does no network IO.
    let message = serde_json::to_string(&inputs)?;
    let mut vault = serde_json::from_str::<PreparedInputs>(&message)?.into_vault();
    let output = vault.render_html(
        "preview.not",
        "#import \"./lib.notc\" as widgets;\n#widgets.panel(title: \"Preview\")[ #widgets.badge(\"nested\") ]",
        RenderOptions::default(),
    )?;
    assert!(output.analysis.diagnostics().is_empty());
    assert!(output.evaluation.diagnostics.is_empty());
    assert!(output.rendered.diagnostics.is_empty());
    println!("{}", output.rendered.html);
    for component in &output.rendered.used_components {
        println!("{}: {}", component.tag, component.module.url().unwrap());
    }
    println!("{} source mappings", output.rendered.source_map.len());
    Ok(())
}
