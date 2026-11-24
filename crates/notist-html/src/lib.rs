//! Shared HTML escaping, URL safety checks, status stylesheet, and optional
//! build-time syntax highlighting for Notist HTML backends.

mod escape;
#[cfg(feature = "highlight")]
pub mod highlight;
mod url;

pub use escape::{escape_attribute, escape_text};
pub use url::is_safe_url;

/// Optional CSS for draft, staged and outdate status annotations.
pub const STATUS_CSS: &str = include_str!("status.css");

/// Extract the leading language token from a fenced code block info string.
/// Fence info strings can carry comma- or whitespace-separated attributes such
/// as `rust,ignore,no_run`. Only the leading language controls presentation.
pub fn code_language(info: &str) -> Option<&str> {
    info.trim_start()
        .split(|ch: char| ch == ',' || ch.is_whitespace())
        .next()
        .filter(|language| !language.is_empty())
}
