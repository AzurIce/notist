use rowan::NodeOrToken;

use notist_core::expr::{BodyFlavor, Expr};
use notist_syntax::syntax::{SyntaxKind, SyntaxNode, SyntaxToken};
use notist_syntax::{ast, parser};

use crate::item::Item;
use crate::{desugar, materialize};

pub fn analyze_json(src: &str) -> String {
    let parse = parser::parse(src);
    let mut out = String::from("{\"tree\":");
    write_element(&mut out, NodeOrToken::Node(parse.syntax()));
    let mut diagnostics: Vec<crate::diag::Diagnostic> = parse
        .diagnostics
        .iter()
        .map(|d| crate::diag::Diagnostic::new(crate::diag::Phase::Syntax, d.span, d.message.clone()))
        .collect();
    if let Some(document) = ast::Document::cast(parse.syntax()) {
        out.push_str(",\"ast\":");
        write_ast(&mut out, &document);
        let (exprs, meta) = desugar::desugar(&document, &mut diagnostics);
        let range = document.range();
        out.push_str(",\"ir1\":");
        write_forest(&mut out, range, &exprs);
        let exprs = crate::resolve::resolve(exprs, &mut diagnostics);
        let exprs = crate::shape::shape(exprs);
        out.push_str(",\"ir2\":");
        write_forest(&mut out, range, &exprs);
        out.push_str(",\"core\":");
        let item = materialize::materialize_doc(&exprs, range, meta);
        write_item(&mut out, &item);
    }
    out.push_str(",\"diagnostics\":[");
    for (i, d) in diagnostics.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"start\":{},\"end\":{},\"phase\":\"{}\",\"message\":{}}}",
            u32::from(d.span.start()),
            u32::from(d.span.end()),
            d.phase,
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

fn write_ast(out: &mut String, document: &ast::Document) {
    let range = document.range();
    out.push_str(&format!(
        "{{\"kind\":\"Document\",\"start\":{},\"end\":{},\"children\":[",
        u32::from(range.start()),
        u32::from(range.end()),
    ));
    let elements: Vec<_> = document.elements().collect();
    let mut first = true;
    let mut run: Vec<NodeOrToken<SyntaxNode, SyntaxToken>> = Vec::new();
    for i in 0..elements.len() {
        let el = elements[i].clone();
        match &el {
            NodeOrToken::Node(node)
                if matches!(
                    node.kind(),
                    SyntaxKind::Heading
                        | SyntaxKind::List
                        | SyntaxKind::Raw
                        | SyntaxKind::Annotation
                ) =>
            {
                write_ast_run(out, &mut run, &mut first);
                if !first {
                    out.push(',');
                }
                first = false;
                match node.kind() {
                    SyntaxKind::Heading => {
                        let heading = ast::Heading::cast(node.clone()).unwrap();
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
                    SyntaxKind::Annotation => {
                        let annotation = ast::Annotation::cast(node.clone()).unwrap();
                        let range = annotation.range();
                        let (text, _) = annotation.payload();
                        out.push_str(&format!(
                            "{{\"kind\":\"Annotation\",\"start\":{},\"end\":{},\"label\":{}}}",
                            u32::from(range.start()),
                            u32::from(range.end()),
                            escape(&format!(
                                "{}({text})",
                                if annotation.is_module() { "@!" } else { "@" }
                            )),
                        ));
                    }
                    SyntaxKind::List => {
                        write_ast_list(out, &ast::List::cast(node.clone()).unwrap())
                    }
                    SyntaxKind::Raw => {
                        let range = node.text_range();
                        out.push_str(&format!(
                            "{{\"kind\":\"Raw\",\"start\":{},\"end\":{}}}",
                            u32::from(range.start()),
                            u32::from(range.end()),
                        ));
                    }
                    _ => unreachable!(),
                }
            }
            // ParBreak / Error: boundaries without output
            NodeOrToken::Node(node)
                if matches!(node.kind(), SyntaxKind::ParBreak | SyntaxKind::Error) =>
            {
                write_ast_run(out, &mut run, &mut first)
            }
            NodeOrToken::Token(t)
                if t.kind() == SyntaxKind::Newline && {
                    let mut j = i + 1;
                    while matches!(
                        elements.get(j).map(|e| e.kind()),
                        Some(SyntaxKind::Whitespace)
                    ) {
                        j += 1;
                    }
                    matches!(
                        elements.get(j).map(|e| e.kind()),
                        None | Some(SyntaxKind::Newline)
                    )
                } =>
            {
                write_ast_run(out, &mut run, &mut first);
            }
            _ => run.push(el),
        }
    }
    write_ast_run(out, &mut run, &mut first);
    out.push_str("]}");
}

/// A maximal run of inline content, displayed as an Inline group of Lines.
/// Lines are split at top-level newline tokens; inline nodes stay whole.
/// (Not a paragraph — paragraphs are formed at desugar, this view is syntax.)
fn write_ast_run(
    out: &mut String,
    run: &mut Vec<NodeOrToken<SyntaxNode, SyntaxToken>>,
    first: &mut bool,
) {
    fn range_of(el: &NodeOrToken<SyntaxNode, SyntaxToken>) -> (u32, u32) {
        match el {
            NodeOrToken::Node(n) => {
                let r = n.text_range();
                (u32::from(r.start()), u32::from(r.end()))
            }
            NodeOrToken::Token(t) => {
                let r = t.text_range();
                (u32::from(r.start()), u32::from(r.end()))
            }
        }
    }
    let mut lines: Vec<Vec<NodeOrToken<SyntaxNode, SyntaxToken>>> = Vec::new();
    let mut current: Vec<NodeOrToken<SyntaxNode, SyntaxToken>> = Vec::new();
    for el in run.drain(..) {
        if matches!(&el, NodeOrToken::Token(t) if t.kind() == SyntaxKind::Newline) {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
        } else {
            current.push(el);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        return;
    }
    if !*first {
        out.push(',');
    }
    *first = false;
    let start = range_of(lines.first().unwrap().first().unwrap()).0;
    let end = range_of(lines.last().unwrap().last().unwrap()).1;
    out.push_str(&format!("{{\"kind\":\"Inline\",\"start\":{start},\"end\":{end},\"children\":["));
    for (j, line) in lines.iter().enumerate() {
        if j > 0 {
            out.push(',');
        }
        let start = range_of(line.first().unwrap()).0;
        let end = range_of(line.last().unwrap()).1;
        out.push_str(&format!(
            "{{\"kind\":\"Line\",\"start\":{start},\"end\":{end},\"children\":["
        ));
        for (k, el) in line.iter().enumerate() {
            if k > 0 {
                out.push(',');
            }
            write_element(out, el.clone());
        }
        out.push_str("]}");
    }
    out.push_str("]}");
}

fn write_ast_list_item(out: &mut String, item: &ast::ListItem) {
    let range = item.range();
    out.push_str(&format!(
        "{{\"kind\":\"ListItem\",\"start\":{},\"end\":{},\"children\":[",
        u32::from(range.start()),
        u32::from(range.end()),
    ));
    let mut first = true;
    for element in item.content() {
        if !first {
            out.push(',');
        }
        first = false;
        write_element(out, element);
    }
    for nested in item.lists() {
        if !first {
            out.push(',');
        }
        first = false;
        write_ast_list(out, &nested);
    }
    out.push_str("]}");
}

fn write_ast_list(out: &mut String, list: &ast::List) {
    let range = list.range();
    let ordered = list.items().next().and_then(|item| item.marker()) == Some(SyntaxKind::Plus);
    out.push_str(&format!(
        "{{\"kind\":\"List\",\"start\":{},\"end\":{},\"label\":{},\"children\":[",
        u32::from(range.start()),
        u32::from(range.end()),
        escape(&format!("ordered = {ordered}")),
    ));
    for (j, item) in list.items().enumerate() {
        if j > 0 {
            out.push(',');
        }
        write_ast_list_item(out, &item);
    }
    out.push_str("]}");
}

fn write_forest<N: std::fmt::Display>(out: &mut String, range: rowan::TextRange, exprs: &[Expr<N>]) {
    out.push_str(&format!(
        "{{\"kind\":\"Document\",\"start\":{},\"end\":{},\"children\":[",
        u32::from(range.start()),
        u32::from(range.end()),
    ));
    for (i, expr) in exprs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_expr(out, expr);
    }
    out.push_str("]}");
}

fn write_expr<N: std::fmt::Display>(out: &mut String, expr: &Expr<N>) {
    let span = expr.span();
    let (start, end) = (u32::from(span.start()), u32::from(span.end()));
    match expr {
        Expr::Literal(value, _) => {
            out.push_str(&format!(
                "{{\"kind\":\"Literal\",\"start\":{start},\"end\":{end},\"label\":{}}}",
                escape(&value.to_string())
            ));
        }
        Expr::Call {
            name,
            args,
            fields,
            children,
            body,
            attrs,
            ..
        } => {
            let mut label = name.to_string();
            for (key, value) in fields.iter() {
                label.push_str(&format!(" :{key} {value}"));
            }
            for (key, value) in attrs.iter() {
                label.push_str(&format!(" @{key} {value}"));
            }
            match body {
                BodyFlavor::Inline => label.push_str(" [inline]"),
                BodyFlavor::Block => label.push_str(" [block]"),
                BodyFlavor::None => {}
            }
            out.push_str(&format!(
                "{{\"kind\":\"Call\",\"start\":{start},\"end\":{end},\"label\":{}",
                escape(&label)
            ));
            if !args.is_empty() || !children.is_empty() {
                out.push_str(",\"children\":[");
                let mut first = true;
                for arg in args {
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    let span = arg.span();
                    out.push_str(&format!(
                        "{{\"kind\":\"Arg\",\"start\":{},\"end\":{},\"children\":[",
                        u32::from(span.start()),
                        u32::from(span.end()),
                    ));
                    write_expr(out, arg);
                    out.push(']');
                    out.push('}');
                }
                for child in children {
                    if !first {
                        out.push(',');
                    }
                    first = false;
                    write_expr(out, child);
                }
                out.push(']');
            }
            out.push('}');
        }
    }
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
    for (key, value) in item.attrs.iter() {
        label.push_str(&format!(" @{key} {value}"));
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
