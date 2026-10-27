//! Native snippets use the lossless CST, including recovered/incomplete input.
//! No evaluation, imports, resource reads or duplicate language grammar.
use crate::escape_text;
use notist_syntax::{
    Parse, code,
    markup::{self, MarkupContentParser},
    tree::{SyntaxKind as K, SyntaxToken},
};

pub(super) fn render(source: &str, module: bool) -> String {
    let parsed = if module {
        code::parse_module(source, &MarkupContentParser)
    } else {
        markup::parse_document(source)
    };
    let mut html = String::from(if module {
        "<span class=\"z-source z-notc\">"
    } else {
        "<span class=\"z-source z-not\">"
    });
    write(&parsed, &mut html);
    html.push_str("</span>");
    html
}

fn write(parsed: &Parse, html: &mut String) {
    for token in parsed
        .syntax()
        .descendants_with_tokens()
        .filter_map(|e| e.into_token())
    {
        // Code retains frontend content trees beside its opaque ContentText
        // tokens. Descend into those trees to highlight embedded Markup too.
        if token.kind() == K::ContentText
            && let Some(content) = parsed
                .contents
                .iter()
                .find(|content| content.body == token.text_range())
                .and_then(|content| content.syntax.as_any().downcast_ref::<Parse>())
        {
            write(content, html);
            continue;
        }
        if let Some(class) = class(&token) {
            html.push_str("<span class=\"");
            html.push_str(class);
            html.push_str("\">");
            html.push_str(&escape_text(token.text()));
            html.push_str("</span>");
        } else {
            html.push_str(&escape_text(token.text()));
        }
    }
}

fn class(token: &SyntaxToken) -> Option<&'static str> {
    // Raw and math payloads are opaque even when their tokens resemble calls,
    // comments or operators. The parser, rather than spelling, owns this boundary.
    let parent = token.parent()?;
    if parent
        .ancestors()
        .any(|node| matches!(node.kind(), K::Raw | K::RawInline | K::Math))
    {
        return Some("z-markup z-raw");
    }
    match token.kind() {
        K::Whitespace | K::Newline => return None,
        K::LineComment | K::BlockComment => return Some("z-comment"),
        K::Str => return Some("z-string"),
        K::Number => return Some("z-constant z-numeric"),
        K::FnKeyword
        | K::ImportKeyword
        | K::AsKeyword
        | K::LetKeyword
        | K::IfKeyword
        | K::ElseKeyword
        | K::MatchKeyword
        | K::ComptimeKeyword => return Some("z-keyword"),
        K::Hash | K::At => return Some("z-keyword"),
        K::Escape => return Some("z-constant z-character z-escape"),
        K::Ident if matches!(token.text(), "true" | "false" | "none") => {
            return Some("z-constant z-language");
        }
        K::Ident if parent.ancestors().any(|node| node.kind() == K::TypeRef) => {
            return Some("z-storage z-type");
        }
        K::Ident if matches!(parent.kind(), K::FunctionDecl | K::Path | K::MemberExpr) => {
            return Some("z-entity z-name z-function");
        }
        K::Eq
        | K::EqEq
        | K::NotEq
        | K::Less
        | K::Greater
        | K::LessEq
        | K::GreaterEq
        | K::AndAnd
        | K::OrOr
        | K::Arrow
        | K::FatArrow
        | K::Plus
        | K::Minus
        | K::Star
        | K::Slash
        | K::Percent
            if matches!(
                parent.kind(),
                K::BinaryExpr
                    | K::UnaryExpr
                    | K::FunctionDecl
                    | K::LetBinding
                    | K::Parameter
                    | K::TypeRef
                    | K::CodeCall
                    | K::MatchArm
            ) =>
        {
            return Some("z-keyword z-operator");
        }
        _ => {}
    }
    for node in parent.ancestors() {
        match node.kind() {
            K::Heading => return Some("z-markup z-heading"),
            K::Strong => return Some("z-markup z-bold"),
            K::Emph => return Some("z-markup z-italic"),
            K::Strike => return Some("z-markup z-strikethrough"),
            K::Link | K::WikiLink | K::Embed => return Some("z-markup z-underline z-link"),
            _ => {}
        }
    }
    None
}
