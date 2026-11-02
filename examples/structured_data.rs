//! `cargo run --example structured_data`: the same body in an immutable batch.
use notist::{MemoryResources, Vault};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut resources = MemoryResources::new("/example");
    for (path, source) in [
        ("views.notc", include_str!("structured-data/views.notc")),
        (
            "private/items.json",
            include_str!("structured-data/private/items.json"),
        ),
        (
            "private/config.toml",
            include_str!("structured-data/private/config.toml"),
        ),
    ] {
        resources.insert(path, source);
    }
    let mut vault = Vault::new(resources);
    for page in ["one.not", "two.md"] {
        let output = vault.render_html(
            page,
            include_str!("structured-data/page.not"),
            Default::default(),
        )?;
        assert!(output.analysis.diagnostics().is_empty());
        assert!(output.evaluation.diagnostics.is_empty());
        assert!(output.rendered.diagnostics.is_empty());
        assert!(!output.rendered.html.contains("private_note"));
        println!("{}", output.rendered.html);
    }
    assert_eq!(vault.input_session().data_cache_stats().parses, 2);
    Ok(())
}
