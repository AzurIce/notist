use rowan::TextRange;

use crate::diag::{Diagnostic, Phase};
use crate::item::{Ctor, Dict, Value};

/// Validate builtin data contracts without discarding the user's invalid
/// values, so recovery trees and debug views retain the original input.
pub(super) fn validate(
    ctor: &Ctor,
    fields: &mut Dict,
    span: TextRange,
    diags: &mut Vec<Diagnostic>,
) {
    let name = ctor.source_name().unwrap_or("custom");
    let rules: &[(&str, &str, bool)] = match ctor {
        Ctor::Text | Ctor::Math => &[("text", "string", true)],
        Ctor::RawInline => &[
            ("text", "string", true),
            ("block", "boolean", false),
            ("lang", "string", false),
        ],
        Ctor::Link => &[("target", "string", true), ("title", "string", false)],
        Ctor::Embed => &[
            ("target", "string", true),
            ("description", "string", false),
            ("title", "string", false),
        ],
        Ctor::Heading => &[("level", "positive integer", false)],
        Ctor::List => &[("ordered", "boolean", false), ("start", "integer", false)],
        Ctor::Callout => &[("kind", "string", false)],
        Ctor::Table => &[("align", "alignment array", false)],
        Ctor::TableRow => &[("header", "boolean", false)],
        _ => &[],
    };
    for &(field, expected, required) in rules {
        let valid = match fields.get(field) {
            None if !required => continue,
            None => {
                diags.push(Diagnostic::new(Phase::Type, span, format!("`{name}` requires `{field}`")));
                continue;
            }
            Some(value) => match (expected, value) {
                ("string", Value::Str(_)) | ("boolean", Value::Bool(_)) | ("integer", Value::Int(_)) => true,
                ("positive integer", Value::Int(level)) => *level > 0,
                ("alignment array", Value::Array(values)) => values.iter().all(|value|
                    matches!(value, Value::Str(align) if matches!(align.as_str(), "none" | "left" | "center" | "right"))),
                _ => false,
            },
        };
        if !valid {
            diags.push(Diagnostic::new(
                Phase::Type,
                span,
                format!("`{name}.{field}` must be {expected}"),
            ));
        }
    }
    match ctor {
        Ctor::List => {
            if fields.get("ordered").is_none() {
                fields.insert("ordered", Value::Bool(false));
            }
            if fields.get("start").is_none() {
                fields.insert("start", Value::Int(1));
            }
        }
        Ctor::Heading if fields.get("level").is_none() => fields.insert("level", Value::Int(1)),
        _ => {}
    }
}
