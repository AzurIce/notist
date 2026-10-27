//! Optional build-time code highlighting. Enable the `highlight` Cargo feature
//! and install [`code`] with [`crate::Renderer::with_code_renderer`]. The host
//! publishes [`CSS`] or supplies its own styles for the `z-` scope classes.
use notist_core::item::{Item, Value};
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

/// Highlight a raw node's text without replacing its pre/code shell. Only the
/// first whitespace-separated language tag is used; detection is never implicit.
/// Missing, unknown and explicit plain-text languages select escaped fallback.
pub fn code(item: &Item) -> Option<String> {
    let Value::Str(language) = item.fields.get("lang")? else {
        return None;
    };
    let Value::Str(text) = item.fields.get("text")? else {
        return None;
    };
    let language = language.split_whitespace().next()?.to_ascii_lowercase();
    match language.as_str() {
        "not" | "notist" => return Some(native::render(text, false)),
        "notc" => return Some(native::render(text, true)),
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
