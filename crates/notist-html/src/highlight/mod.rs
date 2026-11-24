//! Optional build-time code highlighting. Enable the `highlight` Cargo feature
//! and call [`highlight_str`]. The host publishes [`CSS`] or supplies its own
//! styles for the `z-` scope classes.
use std::sync::OnceLock;
use syntect::{
    html::{ClassStyle, ClassedHTMLGenerator},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

mod native;

/// Shared light/dark palette. Explicit `data-theme="light"` / `"dark"` on the
/// document root overrides the system preference. No script is required.
pub const CSS: &str = include_str!("highlight.css");

/// Highlight code text for a language tag without replacing the surrounding
/// `<pre><code>` shell. Only the first comma- or whitespace-separated language
/// tag is used; missing, unknown, and plain-text languages return `None`.
pub fn highlight_str(language: &str, text: &str) -> Option<String> {
    let language = crate::code_language(language)?.to_ascii_lowercase();
    match language.as_str() {
        "not" | "notc" | "notist" => return Some(native::render(text, true)),
        "text" | "txt" | "plaintext" | "plain" | "none" | "nohighlight" => return None,
        _ => {}
    }
    let language = match language.as_str() {
        "shell" => "bash",
        "docker" => "dockerfile",
        language => language,
    };
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    let syntaxes = SYNTAXES.get_or_init(two_face::syntax::extra_newlines);
    let syntax = syntaxes.find_syntax_by_token(language)?;
    let mut html = ClassedHTMLGenerator::new_with_class_style(
        syntax,
        syntaxes,
        ClassStyle::SpacedPrefixed { prefix: "z-" },
    );
    for line in LinesWithEndings::from(text) {
        html.parse_html_for_line_which_includes_newline(line).ok()?;
    }
    Some(html.finalize())
}
