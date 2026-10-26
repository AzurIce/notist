use notist::{Item, Value};
use std::sync::OnceLock;
use syntect::{
    html::{ClassStyle, ClassedHTMLGenerator},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

pub(crate) fn code(item: &Item) -> Option<String> {
    let Some(Value::Str(language)) = item.fields.get("lang") else {
        return None;
    };
    let Some(Value::Str(text)) = item.fields.get("text") else {
        return None;
    };
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    let syntaxes = SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines);
    let syntax = syntaxes.find_syntax_by_token(language.split_whitespace().next()?)?;
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
