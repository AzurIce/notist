//! Lossless Code CST to expressions. Markup content lowers to IR₁ here;
//! the shared content backend runs later in each concrete lexical environment.
use super::{
    ContentLowering,
    literals::{key_text, syntax_value},
};
use notist_core::{
    diag::{Diagnostic, Phase},
    expr::{BodyFlavor, CallArg, Expr},
    item::{Dict, Value},
};
use notist_syntax::{
    Parse,
    content::ContentFlavor,
    tree::{SyntaxKind as K, SyntaxNode, SyntaxToken},
};
use rowan::NodeOrToken;

type Element = NodeOrToken<SyntaxNode, SyntaxToken>;
fn significant(e: &Element) -> bool {
    !matches!(
        e.kind(),
        K::Whitespace | K::Newline | K::LineComment | K::BlockComment | K::Comma
    )
}
fn expressions(node: &SyntaxNode) -> Vec<Element> {
    node.children_with_tokens()
        .filter(significant)
        .filter(|e| {
            if matches!(node.kind(), K::BinaryExpr | K::UnaryExpr) {
                return true;
            }
            !matches!(
                e.kind(),
                K::LParen | K::RParen | K::LBrace | K::RBrace | K::Less | K::Greater
            )
        })
        .collect()
}

pub fn arguments(
    node: &SyntaxNode,
    parsed: Option<&Parse>,
    frontend: Option<&dyn ContentLowering>,
    diags: &mut Vec<Diagnostic>,
) -> Vec<CallArg> {
    expressions(node)
        .into_iter()
        .filter_map(|e| {
            if let NodeOrToken::Node(n) = &e
                && n.kind() == K::Entry
            {
                let entry = notist_syntax::code::ast::Entry::cast(n.clone())?;
                return Some(CallArg::Named {
                    name: key_text(&entry.key_token()?, diags)?,
                    value: expression(entry.value()?, parsed, frontend, diags)?,
                });
            }
            Some(CallArg::Positional(expression(e, parsed, frontend, diags)?))
        })
        .collect()
}

pub fn expression(
    el: Element,
    parsed: Option<&Parse>,
    frontend: Option<&dyn ContentLowering>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Expr> {
    let span = el.text_range();
    let NodeOrToken::Node(node) = &el else {
        return syntax_value(&el, diags).map(|v| Expr::Literal(v, span));
    };
    let parts = expressions(node);
    let child = |e: Element, d: &mut Vec<Diagnostic>| expression(e, parsed, frontend, d);
    let at = |i: usize, d: &mut Vec<Diagnostic>| child(parts.get(i)?.clone(), d);
    Some(match node.kind() {
        K::Path => {
            let name = node.text().to_string();
            match name.as_str() {
                "true" => Expr::Literal(Value::Bool(true), span),
                "false" => Expr::Literal(Value::Bool(false), span),
                _ => Expr::reference(name, span),
            }
        }
        K::MemberExpr => {
            let base = child(
                node.children_with_tokens()
                    .find(|e| e.kind() != K::Whitespace)?,
                diags,
            )?;
            let member = node
                .children_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|t| t.kind() == K::Ident)?;
            Expr::Member {
                base: Box::new(base),
                name: member.text().into(),
                span,
            }
        }
        K::ContentLiteral => {
            let Some(parsed) = parsed else {
                diags.push(Diagnostic::new(
                    Phase::Type,
                    span,
                    "content is mounted after the call, not passed as a data argument",
                ));
                return None;
            };
            let content = parsed.contents.iter().find(|c| c.range == span)?;
            let lowered = frontend?.lower_content(content, diags);
            Expr::Content {
                forest: lowered.forest,
                annotations: lowered.annotations,
                body: if content.flavor == ContentFlavor::Inline {
                    BodyFlavor::Inline
                } else {
                    BodyFlavor::Block
                },
                span,
            }
        }
        K::BlockExpr => {
            let mut bindings = Vec::new();
            let mut value = None;
            for e in parts {
                if let NodeOrToken::Node(n) = &e
                    && n.kind() == K::LetBinding
                {
                    let name = n
                        .children_with_tokens()
                        .filter_map(|e| e.into_token())
                        .find(|t| t.kind() == K::Ident)?
                        .text()
                        .to_string();
                    let val = n
                        .children_with_tokens()
                        .skip_while(|e| e.kind() != K::Eq)
                        .skip(1)
                        .find(significant)?;
                    bindings.push((name, child(val, diags)?));
                    continue;
                }
                value = Some(child(e, diags)?);
            }
            Expr::Block {
                bindings,
                value: Box::new(value.unwrap_or(Expr::Literal(Value::Unit, span))),
                span,
            }
        }
        K::IfExpr => {
            let p: Vec<_> = parts
                .into_iter()
                .filter(|e| !matches!(e.kind(), K::IfKeyword | K::ElseKeyword))
                .collect();
            Expr::If {
                condition: Box::new(child(p.first()?.clone(), diags)?),
                yes: Box::new(child(p.get(1)?.clone(), diags)?),
                no: Box::new(child(p.get(2)?.clone(), diags)?),
                span,
            }
        }
        K::MatchExpr => {
            let subject = parts
                .iter()
                .find(|e| !matches!(e.kind(), K::MatchKeyword | K::MatchArm))?
                .clone();
            let mut arms = Vec::new();
            for arm in node.children().filter(|n| n.kind() == K::MatchArm) {
                let p = expressions(&arm);
                let p: Vec<_> = p.into_iter().filter(|e| e.kind() != K::FatArrow).collect();
                arms.push((
                    child(p.first()?.clone(), diags)?,
                    child(p.get(1)?.clone(), diags)?,
                ));
            }
            Expr::Match {
                subject: Box::new(child(subject, diags)?),
                arms,
                span,
            }
        }
        K::ComptimeExpr => Expr::Comptime {
            value: Box::new(child(
                parts.into_iter().find(|e| e.kind() != K::ComptimeKeyword)?,
                diags,
            )?),
            span,
        },
        K::BinaryExpr => Expr::Binary {
            op: parts.get(1)?.to_string(),
            left: Box::new(at(0, diags)?),
            right: Box::new(at(2, diags)?),
            span,
        },
        K::UnaryExpr
            if parts.first().is_some_and(|e| e.to_string() == "-")
                && parts
                    .get(1)
                    .is_some_and(|e| e.kind() == K::Number && !e.to_string().contains('.')) =>
        {
            let text = format!("-{}", parts.get(1)?);
            match text.parse::<i64>() {
                Ok(n) => Expr::Literal(Value::Int(n), span),
                Err(_) => {
                    diags.push(Diagnostic::new(Phase::Type, span, "number out of range"));
                    return None;
                }
            }
        }
        K::UnaryExpr => Expr::Unary {
            op: parts.first()?.to_string(),
            value: Box::new(at(1, diags)?),
            span,
        },
        K::InvokeExpr => {
            let callee = child(parts.first()?.clone(), diags)?;
            let mut args = Vec::new();
            let mut comptime = Vec::new();
            let mut forest = Vec::new();
            let mut body = BodyFlavor::None;
            for n in node.children().skip(1) {
                match n.kind() {
                    K::ArgumentList => args = arguments(&n, parsed, frontend, diags),
                    K::ComptimeArguments => comptime = arguments(&n, parsed, frontend, diags),
                    K::ContentLiteral => {
                        let content = parsed?
                            .contents
                            .iter()
                            .find(|c| c.range == n.text_range())?;
                        body = if content.flavor == ContentFlavor::Inline {
                            BodyFlavor::Inline
                        } else {
                            BodyFlavor::Block
                        };
                        forest.push(child(NodeOrToken::Node(n), diags)?);
                    }
                    _ => {}
                }
            }
            Expr::Call {
                callee: Box::new(callee),
                args,
                comptime,
                children: forest,
                body,
                attrs: Dict::default(),
                span,
            }
        }
        K::Array => Expr::Array(
            parts
                .into_iter()
                .map(|e| child(e, diags))
                .collect::<Option<_>>()?,
            span,
        ),
        K::Dict => {
            let mut values = Vec::new();
            for e in parts {
                if e.kind() == K::Colon {
                    continue;
                }
                let NodeOrToken::Node(n) = e else {
                    diags.push(Diagnostic::new(
                        Phase::Type,
                        span,
                        "dict members require keys",
                    ));
                    return None;
                };
                let en = notist_syntax::code::ast::Entry::cast(n)?;
                values.push((
                    key_text(&en.key_token()?, diags)?,
                    child(en.value()?, diags)?,
                ));
            }
            Expr::Dict(values, span)
        }
        K::Group => return at(0, diags),
        _ => return syntax_value(&el, diags).map(|v| Expr::Literal(v, span)),
    })
}
