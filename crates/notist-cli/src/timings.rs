use notist::progress::timings::Report;
use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum Format {
    Html,
    Json,
}

/// Reports live outside publication artifacts; timestamped names preserve failures.
pub(crate) fn write(report: &Report, formats: &[Format]) -> std::io::Result<()> {
    let directory = PathBuf::from("target/notist-timings");
    std::fs::create_dir_all(&directory)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for format in formats {
        let (extension, content) = match format {
            Format::Html => ("html", report.html()),
            Format::Json => ("json", report.json()),
        };
        let path = directory.join(format!("timings-{stamp}.{extension}"));
        std::fs::write(&path, content)?;
        eprintln!("Timings: {}", path.display());
    }
    Ok(())
}
