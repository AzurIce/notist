//! One build's diagnostics, shared by the terminal and live preview.
use super::{BuildOutput, Error};
use codespan_reporting::{diagnostic::Severity, term::termcolor::Buffer};
use notist::SourceDiagnostic;
use serde::Serialize;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Status {
    Building,
    Ready,
    Warnings,
    Errors,
    Failed,
}

#[derive(Clone, Serialize)]
pub(super) struct Issue {
    pub message: String,
    pub detail: String,
    #[serde(skip)]
    pub(super) source: Option<SourceDiagnostic>,
}

impl Issue {
    fn source(source: &SourceDiagnostic, severity: Severity) -> Self {
        let mut buffer = Buffer::no_color();
        crate::write_source(
            &mut buffer,
            &source.path,
            &source.source,
            std::slice::from_ref(&source.diagnostic),
            severity,
        );
        Self {
            message: source.diagnostic.message.clone(),
            detail: String::from_utf8(buffer.into_inner()).unwrap(),
            source: Some(source.clone()),
        }
    }
}

#[derive(Clone, Serialize)]
pub(super) struct Report {
    pub build: u64,
    pub status: Status,
    pub elapsed_ms: u128,
    pub pages: usize,
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
    pub resolved: bool,
}

impl Default for Report {
    fn default() -> Self {
        Self {
            build: 0,
            status: Status::Building,
            elapsed_ms: 0,
            pages: 0,
            errors: vec![],
            warnings: vec![],
            resolved: false,
        }
    }
}

impl Report {
    pub fn begin(&mut self) {
        self.build += 1;
        self.status = Status::Building;
        self.elapsed_ms = 0;
        self.resolved = false;
        // Keep previous diagnostics until this build has checked them.
    }

    pub fn success(&self, site: &BuildOutput, elapsed: Duration) -> Self {
        let status = if !site.diagnostics.is_empty() {
            Status::Errors
        } else if !site.warnings.is_empty() {
            Status::Warnings
        } else {
            Status::Ready
        };
        Self {
            build: self.build,
            status,
            elapsed_ms: elapsed.as_millis(),
            pages: site.pages,
            errors: site
                .diagnostics
                .iter()
                .map(|d| Issue::source(d, Severity::Error))
                .collect(),
            warnings: site
                .warnings
                .iter()
                .map(|d| Issue::source(d, Severity::Warning))
                .collect(),
            resolved: !self.errors.is_empty() && site.diagnostics.is_empty(),
        }
    }

    pub fn failure(&self, error: &Error, elapsed: Duration) -> Self {
        let errors = match error {
            Error::Diagnostics(errors) => errors
                .iter()
                .map(|d| Issue::source(d, Severity::Error))
                .collect(),
            error => vec![Issue {
                message: error.to_string(),
                detail: error.to_string(),
                source: None,
            }],
        };
        Self {
            build: self.build,
            status: Status::Failed,
            elapsed_ms: elapsed.as_millis(),
            pages: self.pages,
            errors,
            warnings: vec![],
            resolved: false,
        }
    }

    pub fn summary(&self, has_preview: bool) -> String {
        let errors = count(self.errors.len(), "error");
        let warnings = count(self.warnings.len(), "warning");
        match self.status {
            Status::Building => "Rebuilding…".into(),
            Status::Ready => format!("✓ Updated · {}", count(self.pages, "page")),
            Status::Warnings => format!("⚠ Updated · {warnings}"),
            Status::Errors => format!("✗ Preview updated with {errors} · {warnings}"),
            Status::Failed => format!(
                "✗ Build failed · {errors} · {}",
                if has_preview {
                    "showing previous preview"
                } else {
                    "no preview available yet"
                }
            ),
        }
    }
}

fn count(value: usize, noun: &str) -> String {
    format!("{value} {noun}{}", if value == 1 { "" } else { "s" })
}
