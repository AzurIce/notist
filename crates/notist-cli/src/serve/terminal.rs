use super::report::{Report, Status};
use codespan_reporting::{
    diagnostic::Severity,
    term::termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor},
};
use std::{
    io::{IsTerminal, Write},
    time::Instant,
};

pub(super) struct Terminal {
    url: String,
    clear: bool,
    started: Instant,
}

impl Terminal {
    pub fn new(url: String, no_clear_screen: bool) -> Self {
        Self {
            url,
            clear: !no_clear_screen
                && std::io::stderr().is_terminal()
                && std::io::stdout().is_terminal()
                && std::env::var_os("CI").is_none()
                && std::env::var("TERM").as_deref() != Ok("dumb"),
            started: Instant::now(),
        }
    }

    pub fn render(&self, report: &Report, has_preview: bool) {
        let stream = StandardStream::stderr(ColorChoice::Auto);
        let mut writer = stream.lock();
        if self.clear {
            // Clear the viewport, preserving terminal scrollback.
            let _ = write!(writer, "\x1b[2J\x1b[H");
        }
        let _ = writeln!(
            writer,
            "[notist #{} +{:.1}s] {}",
            report.build,
            self.started.elapsed().as_secs_f32(),
            self.url
        );
        let color = match report.status {
            Status::Building => Color::Cyan,
            Status::Ready => Color::Green,
            Status::Warnings => Color::Yellow,
            Status::Errors | Status::Failed => Color::Red,
        };
        let _ = writer.set_color(ColorSpec::new().set_fg(Some(color)).set_bold(true));
        let _ = write!(writer, "{}", report.summary(has_preview));
        let _ = writer.reset();
        if report.status != Status::Building {
            let _ = write!(writer, " · {}ms", report.elapsed_ms);
        }
        let _ = writeln!(writer);
        if report.resolved {
            let _ = writer.set_color(ColorSpec::new().set_fg(Some(Color::Green)));
            let _ = writeln!(writer, "✓ All errors resolved");
            let _ = writer.reset();
        }
        if report.status == Status::Building
            && (!report.errors.is_empty() || !report.warnings.is_empty())
        {
            let _ = writeln!(writer, "Checking previous diagnostics…");
        }
        let _ = writer.flush();
        drop(writer);
        if report.status != Status::Building {
            for (severity, issues) in [
                (Severity::Error, &report.errors),
                (Severity::Warning, &report.warnings),
            ] {
                for issue in issues {
                    if let Some(source) = &issue.source {
                        crate::book::report_diagnostics(std::slice::from_ref(source), severity);
                    } else {
                        eprintln!("{}", issue.detail);
                    }
                }
            }
        }
    }
}
