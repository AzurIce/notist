use notist::progress::timings::Report;
use std::{
    fs::File,
    io::{BufWriter, Write},
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
        let extension = match format {
            Format::Html => "html",
            Format::Json => "json",
        };
        let path = directory.join(format!("timings-{stamp}.{extension}"));
        let mut writer = BufWriter::with_capacity(1 << 20, File::create(&path)?);
        match format {
            Format::Html => report.write_html(&mut writer)?,
            Format::Json => report.write_json(&mut writer)?,
        }
        writer.flush()?;
        eprintln!("Timings: {}", path.display());
    }
    Ok(())
}
