use rowan::NodeOrToken;

use notist_syntax::syntax::{SyntaxNode, SyntaxToken};
use notist_syntax::{ast, parser};

use crate::item::Item;
use crate::lower;

pub fn analyze_json(src: &str) -> String {
    let parse = parser::parse(src);
    let mut out = String::from("{\"tree\":");
    write_element(&mut out, NodeOrToken::Node(parse.syntax()));
    if let Some(document) = ast::Document::cast(parse.syntax()) {
        out.push_str(",\"ast\":");
        write_ast(&mut out, &document);
        out.push_str(",\"core\":");
        write_item(&mut out, &lower::lower(&document));
    }
    out.push_str(",\"diagnostics\":[");
    for (i, d) in parse.diagnostics.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"start\":{},\"end\":{},\"message\":{}}}",
            u32::from(d.span.start()),
            u32::from(d.span.end()),
            escape(&d.message),
        ));
    }
    out.push_str("]}");
    out
}

fn write_element(out: &mut String, element: NodeOrToken<SyntaxNode, SyntaxToken>) {
    match element {
        NodeOrToken::Node(node) => {
            let range = node.text_range();
            out.push_str(&format!(
                "{{\"kind\":\"{:?}\",\"start\":{},\"end\":{},\"children\":[",
                node.kind(),
                u32::from(range.start()),
                u32::from(range.end()),
            ));
            for (i, child) in node.children_with_tokens().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_element(out, child);
            }
            out.push_str("]}");
        }
        NodeOrToken::Token(token) => {
            let range = token.text_range();
            out.push_str(&format!(
                "{{\"kind\":\"{:?}\",\"start\":{},\"end\":{},\"text\":{}}}",
                token.kind(),
                u32::from(range.start()),
                u32::from(range.end()),
                escape(token.text()),
            ));
        }
    }
}

fn write_ast(out: &mut String, document: &ast::Document) -> () {
    let range = document.range();
    out.push_str(&format!(
        "{{\"kind\":\"Document\",\"start\":{},\"end\":{},\"children\":[",
        u32::from(range.start()),
        u32::from(range.end()),
    ));
    for (i, block) in document.blocks().enumerate() {
        if i > 0 {
            out.push(',');
        }
        match block {
            ast::Block::Heading(heading) => {
                let range = heading.range();
                out.push_str(&format!(
                    "{{\"kind\":\"Heading\",\"start\":{},\"end\":{},\"label\":{},\"children\":[",
                    u32::from(range.start()),
                    u32::from(range.end()),
                    escape(&format!("level = {}", heading.level())),
                ));
                for (j, token) in heading.content_tokens().iter().enumerate() {
                    if j > 0 {
                        out.push(',');
                    }
                    write_element(out, NodeOrToken::Token(token.clone()));
                }
                out.push_str("]}");
            }
            ast::Block::Paragraph(paragraph) => {
                let range = paragraph.range();
                out.push_str(&format!(
                    "{{\"kind\":\"Paragraph\",\"start\":{},\"end\":{},\"children\":[",
                    u32::from(range.start()),
                    u32::from(range.end()),
                ));
                for (j, line) in paragraph.lines().iter().enumerate() {
                    if j > 0 {
                        out.push(',');
                    }
                    let start = line.tokens.first().map_or(range.start(), |t| t.text_range().start());
                    let end = line.tokens.last().map_or(range.end(), |t| t.text_range().end());
                    out.push_str(&format!(
                        "{{\"kind\":\"Line\",\"start\":{},\"end\":{},\"children\":[",
                        u32::from(start),
                        u32::from(end),
                    ));
                    for (k, token) in line.tokens.iter().enumerate() {
                        if k > 0 {
                            out.push(',');
                        }
                        write_element(out, NodeOrToken::Token(token.clone()));
                    }
                    out.push_str("]}");
                }
                out.push_str("]}");
            }
        }
    }
    out.push_str("]}");
}

fn write_item(out: &mut String, item: &Item) {
    out.push_str(&format!(
        "{{\"kind\":\"{}\",\"start\":{},\"end\":{}",
        item.ctor.name(),
        u32::from(item.span.start()),
        u32::from(item.span.end()),
    ));
    let mut label = String::new();
    for (key, value) in item.fields.iter() {
        label.push_str(&format!(" :{key} {value}"));
    }
    if !label.is_empty() {
        out.push_str(&format!(",\"label\":{}", escape(&label)));
    }
    if !item.children.is_empty() {
        out.push_str(",\"children\":[");
        for (i, child) in item.children.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            write_item(out, child);
        }
        out.push(']');
    }
    out.push('}');
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
