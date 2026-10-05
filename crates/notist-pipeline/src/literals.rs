//! Literal conversion shared by document calls and Code declaration defaults.

use crate::diag::{Diagnostic, Phase};
use crate::item::{Dict, Value};
use notist_syntax::ast::Entry;
use notist_syntax::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};
use rowan::{NodeOrToken, TextRange};

/// code CST → Value. The four string forms are resolved here (escapes, raw,
/// multiline framing).
pub(crate) fn syntax_value(
    element: &NodeOrToken<SyntaxNode, SyntaxToken>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Value> {
    match element {
        NodeOrToken::Token(token) => match token.kind() {
            SyntaxKind::Str => unquote(token.text(), token.text_range(), diags).map(Value::Str),
            SyntaxKind::Number => {
                let text = token.text();
                let parsed = if text.contains('.') {
                    text.parse::<f64>().ok().map(Value::Float)
                } else {
                    text.parse::<i64>().ok().map(Value::Int)
                };
                match parsed {
                    Some(value) => Some(value),
                    None => {
                        diags.push(Diagnostic {
                            phase: Phase::Semantic,
                            span: token.text_range(),
                            message: "number out of range".to_string(),
                        });
                        None
                    }
                }
            }
            SyntaxKind::Ident => match token.text() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => {
                    diags.push(Diagnostic {
                        phase: Phase::Semantic,
                        span: token.text_range(),
                        message: "bare names are not literals".to_string(),
                    });
                    None
                }
            },
            _ => None,
        },
        NodeOrToken::Node(node) => match node.kind() {
            SyntaxKind::Unit => Some(Value::Unit),
            SyntaxKind::Group => {
                let inner: Vec<_> = value_children(node).collect();
                match inner.as_slice() {
                    [el] => syntax_value(el, diags),
                    _ => {
                        diags.push(Diagnostic {
                            phase: Phase::Semantic,
                            span: node.text_range(),
                            message: "expected a single value in grouping".to_string(),
                        });
                        None
                    }
                }
            }
            SyntaxKind::Neg => {
                let number = node
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .find(|t| t.kind() == SyntaxKind::Number)?;
                // Parse the sign with the magnitude so i64::MIN is representable.
                let text = format!("-{}", number.text());
                let parsed = if text.contains('.') {
                    text.parse::<f64>().ok().map(Value::Float)
                } else {
                    text.parse::<i64>().ok().map(Value::Int)
                };
                if parsed.is_none() {
                    diags.push(Diagnostic::new(
                        Phase::Semantic,
                        node.text_range(),
                        "number out of range",
                    ));
                }
                parsed
            }
            SyntaxKind::Array => {
                let mut items = Vec::new();
                for child in value_children(node) {
                    if let Some(value) = syntax_value(&child, diags) {
                        items.push(value);
                    }
                }
                Some(Value::Array(items))
            }
            SyntaxKind::Dict => {
                let mut dict = Dict::default();
                for element in value_children(node) {
                    if element.kind() == SyntaxKind::Colon {
                        continue;
                    }
                    let Some(child) = element.clone().into_node().and_then(Entry::cast) else {
                        diags.push(Diagnostic::new(
                            Phase::Semantic,
                            element.text_range(),
                            "dict members must be `key: value`",
                        ));
                        continue;
                    };
                    let Some(key_token) = child.key_token() else {
                        continue;
                    };
                    let Some(key) = key_text(&key_token, diags) else {
                        continue;
                    };
                    let Some(value_el) = child.value() else {
                        continue;
                    };
                    if let Some(value) = syntax_value(&value_el, diags) {
                        dict.insert(key, value);
                    }
                }
                Some(Value::Dict(dict))
            }
            _ => None,
        },
    }
}

/// A group node's value members: everything except trivia, commas, and the
/// parens themselves.
pub(crate) fn value_children(
    node: &SyntaxNode,
) -> impl Iterator<Item = NodeOrToken<SyntaxNode, SyntaxToken>> {
    node.children_with_tokens().filter(|el| {
        !matches!(
            el.kind(),
            SyntaxKind::Whitespace
                | SyntaxKind::Newline
                | SyntaxKind::LineComment
                | SyntaxKind::BlockComment
                | SyntaxKind::Comma
                | SyntaxKind::LParen
                | SyntaxKind::RParen
        )
    })
}

pub(crate) fn key_text(token: &SyntaxToken, diags: &mut Vec<Diagnostic>) -> Option<String> {
    match token.kind() {
        SyntaxKind::Ident => Some(token.text().to_string()),
        SyntaxKind::Str => unquote(token.text(), token.text_range(), diags),
        _ => None,
    }
}

/// String token text → value content: strips the raw prefix, quotes, and
/// multiline framing; processes the five escapes for non-raw forms.
fn unquote(text: &str, range: TextRange, diags: &mut Vec<Diagnostic>) -> Option<String> {
    let mut body = text;
    let mut hashes = 0;
    if let Some(rest) = body.strip_prefix('r') {
        hashes = rest.bytes().take_while(|&b| b == b'#').count();
        body = &rest[hashes..];
    }
    let quote_len = if body.starts_with("\"\"\"") { 3 } else { 1 };
    let closer = format!("{}{}", "\"".repeat(quote_len), "#".repeat(hashes));
    if !body.starts_with('"')
        || !body.ends_with(closer.as_str())
        || body.len() < quote_len + closer.len()
    {
        diags.push(Diagnostic {
            phase: Phase::Semantic,
            span: range,
            message: "unclosed string".to_string(),
        });
        return None;
    }
    let mut inner = &body[quote_len..body.len() - closer.len()];
    if quote_len == 3 {
        let Some(stripped) = inner
            .strip_prefix("\r\n")
            .or_else(|| inner.strip_prefix('\n'))
            .or_else(|| inner.strip_prefix('\r'))
        else {
            diags.push(Diagnostic::new(
                Phase::Semantic,
                range,
                "multiline string must start with a newline",
            ));
            return None;
        };
        inner = stripped;
        inner = inner
            .strip_suffix("\r\n")
            .or_else(|| inner.strip_suffix('\n'))
            .or_else(|| inner.strip_suffix('\r'))
            .unwrap_or(inner);
    }
    if hashes > 0 {
        return Some(inner.to_string());
    }
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            _ => {
                diags.push(Diagnostic {
                    phase: Phase::Semantic,
                    span: range,
                    message: "unknown escape".to_string(),
                });
                return None;
            }
        }
    }
    Some(out)
}
