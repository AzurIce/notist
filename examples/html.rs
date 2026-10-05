//! Run with `cargo run --example html -- document.not` (or `.md`).
use std::error::Error;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let path = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("usage: cargo run --example html -- <document.not|document.md>")?,
    );
    let source = std::fs::read_to_string(&path)?;
    let output = notist::Vault::open(".").render_html(
        &path,
        &source,
        notist::RenderOptions { source_map: false },
    )?;
    let analysis = output.analysis;
    let rendered = output.rendered;
    for diagnostic in analysis
        .diagnostics()
        .iter()
        .chain(&output.transformed.diagnostics)
        .chain(&rendered.diagnostics)
    {
        eprintln!(
            "{}: {:?}: {}",
            path.display(),
            diagnostic.span,
            diagnostic.message
        );
    }
    print!("{}", rendered.html);
    Ok(())
}
