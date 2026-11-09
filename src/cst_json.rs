//! Structured inspection data. String APIs serialize only at the transport edge.
use rowan::{NodeOrToken, TextRange};
use serde_json::{Value as Json, json};

use notist_core::expr::{BodyFlavor, CallArg, Expr, RExpr};
use notist_syntax::code::ast as code_ast;
use notist_syntax::markup::ast;
use notist_syntax::tree::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::item::Item;

/// Module inspection keeps the recovered CST available even on semantic errors.
pub fn module_json(src: &str, analysis: &crate::CodeAnalysis) -> String {
    module_value(src, analysis).to_string()
}

pub(crate) fn module_value(src: &str, analysis: &crate::CodeAnalysis) -> Json {
    let parse = notist_syntax::code::parse_module(src, &notist_syntax::markup::MarkupContentParser);
    let mut result = json!({"tree": element_json(NodeOrToken::Node(parse.syntax()))});
    if let Some(module) = code_ast::Module::cast(parse.syntax()) {
        result["ast"] = module_ast(&module);
    }
    if let Some(module) = &analysis.definitions {
        result["functions"] = Json::Array(module.functions.iter().map(|function| {
            let parameters: Vec<_> = function.parameters.iter().map(|parameter| {
                let mut value = json!({
                    "name": parameter.name,
                    "type": type_json(&parameter.ty),
                    "mode": match parameter.mode {
                        crate::ParameterMode::Required => "required",
                        crate::ParameterMode::Optional => "optional",
                        crate::ParameterMode::Default(_) => "default",
                    },
                });
                if let crate::ParameterMode::Default(default) = &parameter.mode {
                    value["default"] = crate::json::value(default);
                    value["typed_default"] = notist_html::components::value_json(default);
                }
                value
            }).collect();
            json!({
                "function": function.id.to_string(),
                "start": u32::from(function.span.start()),
                "end": u32::from(function.span.end()),
                "signature": &src[usize::from(function.span.start())..usize::from(function.span.end())],
                "children": format!("{:?}", function.children),
                "level": function.returns.level().map(|level| if level == crate::builtins::Level::Block { "block" } else { "inline" }),
                "parameters": parameters,
                "code": function.implementation.is_code(),
                "returns": type_json(&function.returns),
                "children_type": function.children_type.as_ref().map(type_json),
                "comptime": function.comptime.iter().map(|p| json!({"name":p.name,"type":type_json(&p.ty),"default":p.default.as_ref().map(|v|v.key())})).collect::<Vec<_>>(),
            })
        }).collect());
    }
    result["diagnostics"] = diagnostics_json(&analysis.diagnostics);
    result
}

fn node(kind: &str, span: TextRange) -> Json {
    json!({"kind":kind,"start":u32::from(span.start()),"end":u32::from(span.end())})
}

fn branch(kind: &str, span: TextRange, children: Vec<Json>) -> Json {
    let mut value = node(kind, span);
    value["children"] = Json::Array(children);
    value
}

fn module_ast(module: &code_ast::Module) -> Json {
    let functions = module.functions().map(|function| {
        let mut children: Vec<_> = function
            .parameters()
            .map(|parameter| {
                let mut children = Vec::new();
                if let Some(ty) = parameter.ty() {
                    children.push(ast_node("TypeRef", ty.syntax(), Vec::new()));
                }
                if let Some(default) = parameter.default_value() {
                    children.push(ast_node("DefaultValue", default.syntax(), Vec::new()));
                }
                ast_node("Parameter", parameter.syntax(), children)
            })
            .collect();
        children.extend(
            function
                .comptime_parameters()
                .map(|parameter| ast_node("ComptimeParameter", parameter.syntax(), Vec::new())),
        );
        if let Some(body) = function.body() {
            children.push(ast_node("Body", &body, Vec::new()));
        }
        if let Some(mount) = function.children_decl() {
            children.push(ast_node("ChildrenDecl", mount.syntax(), Vec::new()));
        }
        if let Some(result) = function.return_type() {
            children.push(ast_node("ReturnType", result.syntax(), Vec::new()));
        }
        ast_node("FunctionDecl", function.syntax(), children)
    });
    let mut children: Vec<_> = module
        .imports()
        .map(|import| {
            ast_node(
                "ImportDecl",
                import.syntax(),
                import
                    .members()
                    .map(|member| ast_node("ImportMember", member.syntax(), Vec::new()))
                    .collect(),
            )
        })
        .collect();
    children.extend(functions);
    children.sort_by_key(|node| node["start"].as_u64());
    branch("Module", module.range(), children)
}

fn ast_node(kind: &str, syntax: &SyntaxNode, children: Vec<Json>) -> Json {
    let mut result = branch(kind, syntax.text_range(), children);
    result["label"] = syntax.text().to_string().into();
    result
}

fn type_json(ty: &crate::Type) -> Json {
    use crate::Type;
    match ty {
        Type::Type => json!({"kind":"Type"}),
        Type::Never => json!({"kind":"Never"}),
        Type::Data => json!({"kind":"Data"}),
        Type::DataError => json!({"kind":"DataError"}),
        Type::InlineContent => json!({"kind":"InlineContent"}),
        Type::BlockContent => json!({"kind":"BlockContent"}),
        Type::Parameter(name) => json!({"kind":"Parameter","name":name}),
        Type::Unit => json!({"kind":"Unit"}),
        Type::Bool => json!({"kind":"Bool"}),
        Type::Int => json!({"kind":"Int"}),
        Type::Float => json!({"kind":"Float"}),
        Type::String => json!({"kind":"String"}),
        Type::Path => json!({"kind":"Path"}),
        Type::ResourceTarget => json!({"kind":"ResourceTarget"}),
        Type::Dict => json!({"kind":"Dict"}),
        Type::Array(element) => json!({"kind":"Array","element":element.as_deref().map(type_json)}),
        Type::Option(element) => json!({"kind":"Option","element":type_json(element)}),
        Type::Result(ok, err) => json!({"kind":"Result","ok":type_json(ok),"err":type_json(err)}),
    }
}

/// Serialize already computed results; never reruns source processing.
pub fn inspection_json(
    analysis: &crate::Analysis,
    inspection: Option<&notist_pipeline::Inspection>,
) -> Result<String, serde_json::Error> {
    Ok(inspection_value(analysis, inspection)?.to_string())
}

pub(crate) fn inspection_value(
    analysis: &crate::Analysis,
    inspection: Option<&notist_pipeline::Inspection>,
) -> Result<Json, serde_json::Error> {
    let mut result = json!({});
    if let Some(inspection) = inspection {
        if let Some(syntax) = inspection
            .syntax
            .as_deref()
            .and_then(|syntax| syntax.as_any().downcast_ref::<SyntaxNode>())
        {
            result["tree"] = element_json(NodeOrToken::Node(syntax.clone()));
            if let Some(document) = ast::Document::cast(syntax.clone()) {
                result["ast"] = document_ast(&document);
            }
        }
        result["ir1"] = branch(
            "Document",
            analysis.root().span,
            inspection.lowered.iter().map(expr_json).collect(),
        );
        result["ir2"] = branch(
            "Document",
            analysis.root().span,
            inspection
                .shaped
                .iter()
                .map(|expr| resolved_json(expr, analysis.sources()))
                .collect::<Result<_, _>>()?,
        );
    }
    result["core"] = item_json(analysis.root(), analysis.sources())?;
    result["diagnostics"] = diagnostics_json(analysis.diagnostics());
    Ok(result)
}

fn diagnostics_json(diagnostics: &[crate::Diagnostic]) -> Json {
    json!(diagnostics.iter().map(|d| json!({"source":d.source,"start":u32::from(d.span.start()),"end":u32::from(d.span.end()),"phase":d.phase.to_string(),"message":d.message})).collect::<Vec<_>>())
}

fn element_json(element: NodeOrToken<SyntaxNode, SyntaxToken>) -> Json {
    match element {
        NodeOrToken::Node(node) => branch(
            &format!("{:?}", node.kind()),
            node.text_range(),
            node.children_with_tokens().map(element_json).collect(),
        ),
        NodeOrToken::Token(token) => {
            let mut value = node(&format!("{:?}", token.kind()), token.text_range());
            value["text"] = token.text().into();
            value
        }
    }
}

fn document_ast(document: &ast::Document) -> Json {
    let elements: Vec<_> = document.elements().collect();
    let mut children = Vec::new();
    let mut run = Vec::new();
    for (i, element) in elements.iter().enumerate() {
        match element {
            NodeOrToken::Node(syntax)
                if matches!(
                    syntax.kind(),
                    SyntaxKind::Heading
                        | SyntaxKind::ImportDecl
                        | SyntaxKind::List
                        | SyntaxKind::Raw
                        | SyntaxKind::Divider
                        | SyntaxKind::Table
                        | SyntaxKind::Annotation
                        | SyntaxKind::BlockAnnotation
                ) =>
            {
                flush_inline(&mut run, &mut children);
                children.push(match syntax.kind() {
                    SyntaxKind::Heading => {
                        let heading = ast::Heading::cast(syntax.clone()).unwrap();
                        let mut value = branch(
                            "Heading",
                            heading.range(),
                            heading
                                .content_tokens()
                                .iter()
                                .map(|token| element_json(NodeOrToken::Token(token.clone())))
                                .collect(),
                        );
                        value["label"] = format!("level = {}", heading.level()).into();
                        value
                    }
                    SyntaxKind::Annotation | SyntaxKind::BlockAnnotation => {
                        let annotation = code_ast::Annotation::cast(syntax.clone()).unwrap();
                        let mut value = node("Annotation", annotation.range());
                        let (text, _) = annotation.payload();
                        value["label"] = format!(
                            "{}({text})",
                            if annotation.is_module() { "@!" } else { "@" }
                        )
                        .into();
                        value
                    }
                    SyntaxKind::List => {
                        let list = ast::List::cast(syntax.clone()).unwrap();
                        let mut value = branch(
                            "List",
                            list.range(),
                            list.items()
                                .map(|item| {
                                    branch(
                                        "ListItem",
                                        item.range(),
                                        item.content().map(element_json).collect(),
                                    )
                                })
                                .collect(),
                        );
                        value["label"] = format!(
                            "ordered = {}",
                            list.items().next().and_then(|item| item.marker())
                                == Some(SyntaxKind::Plus)
                        )
                        .into();
                        value
                    }
                    SyntaxKind::Table | SyntaxKind::ImportDecl => element_json(element.clone()),
                    SyntaxKind::Raw | SyntaxKind::Divider => {
                        node(&format!("{:?}", syntax.kind()), syntax.text_range())
                    }
                    _ => unreachable!(),
                });
            }
            NodeOrToken::Node(node)
                if matches!(node.kind(), SyntaxKind::ParBreak | SyntaxKind::Error) =>
            {
                flush_inline(&mut run, &mut children)
            }
            NodeOrToken::Token(token)
                if token.kind() == SyntaxKind::Newline && {
                    let next = elements[i + 1..]
                        .iter()
                        .find(|element| element.kind() != SyntaxKind::Whitespace);
                    matches!(
                        next.map(|element| element.kind()),
                        None | Some(SyntaxKind::Newline)
                    )
                } =>
            {
                flush_inline(&mut run, &mut children)
            }
            _ => run.push(element.clone()),
        }
    }
    flush_inline(&mut run, &mut children);
    branch("Document", document.range(), children)
}

/// Display a maximal inline run as lines; paragraph classification belongs to Resolve.
fn flush_inline(run: &mut Vec<NodeOrToken<SyntaxNode, SyntaxToken>>, out: &mut Vec<Json>) {
    let mut lines = Vec::new();
    let mut current: Vec<NodeOrToken<SyntaxNode, SyntaxToken>> = Vec::new();
    let flush = |current: &mut Vec<NodeOrToken<SyntaxNode, SyntaxToken>>, lines: &mut Vec<Json>| {
        if let (Some(first), Some(last)) = (current.first(), current.last()) {
            let span = first.text_range().cover(last.text_range());
            lines.push(branch(
                "Line",
                span,
                current.drain(..).map(element_json).collect(),
            ));
        }
    };
    for element in run.drain(..) {
        if matches!(&element, NodeOrToken::Token(token) if token.kind() == SyntaxKind::Newline) {
            flush(&mut current, &mut lines);
        } else {
            current.push(element);
        }
    }
    flush(&mut current, &mut lines);
    if let (Some(first), Some(last)) = (lines.first(), lines.last()) {
        let span = TextRange::new(
            (first["start"].as_u64().unwrap() as u32).into(),
            (last["end"].as_u64().unwrap() as u32).into(),
        );
        out.push(branch("Inline", span, lines));
    }
}

fn body_label(label: &mut String, body: BodyFlavor) {
    match body {
        BodyFlavor::Inline => label.push_str(" [inline]"),
        BodyFlavor::Block => label.push_str(" [block]"),
        BodyFlavor::None => {}
    }
}

fn expr_json(expr: &Expr) -> Json {
    let mut value = node("Expression", expr.span());
    match expr {
        Expr::Fragment { children, .. } => {
            return branch(
                "Fragment",
                expr.span(),
                children.iter().map(expr_json).collect(),
            );
        }
        Expr::Literal(literal, _) => {
            value["kind"] = "Literal".into();
            value["label"] = literal.to_string().into();
        }
        Expr::PathLiteral { text, .. } => {
            value["kind"] = "PathLiteral".into();
            value["label"] = text.as_str().into();
        }
        Expr::Ref { name, .. } => {
            value["kind"] = "Ref".into();
            value["label"] = name.as_str().into();
        }
        Expr::Member { base, name, .. } => {
            value = branch("Member", expr.span(), vec![expr_json(base)]);
            value["label"] = name.as_str().into();
        }
        Expr::Call {
            callee,
            args,
            children,
            body,
            attrs,
            ..
        } => {
            let mut label = expr.callee_path().unwrap_or_else(|| "<expression>".into());
            for (key, value) in attrs.iter() {
                label.push_str(&format!(" @{key} {value}"));
            }
            body_label(&mut label, *body);
            let mut nodes = vec![branch("Callee", callee.span(), vec![expr_json(callee)])];
            nodes.extend(args.iter().map(|arg| {
                let mut value = branch("Arg", arg.value().span(), vec![expr_json(arg.value())]);
                if let CallArg::Named { name, .. } = arg {
                    value["label"] = name.as_str().into();
                }
                value
            }));
            nodes.extend(children.iter().map(expr_json));
            value = branch("Call", expr.span(), nodes);
            value["label"] = label.into();
        }
        other => value["label"] = format!("{other:?}").into(),
    }
    value
}

fn resolved_json(expr: &RExpr, sources: &crate::Sources) -> Result<Json, serde_json::Error> {
    let mut label = expr
        .ctor
        .name(sources.definitions())
        .map_err(<serde_json::Error as serde::ser::Error>::custom)?
        .into_owned();
    for (key, value) in expr.fields.iter() {
        label.push_str(&format!(" :{key} {value}"));
    }
    for (key, value) in expr.attrs.iter() {
        label.push_str(&format!(" @{key} {value}"));
    }
    body_label(&mut label, expr.body);
    let mut value = node("Node", expr.span);
    value["label"] = label.into();
    if !expr.children.is_empty() {
        value["children"] = Json::Array(
            expr.children
                .iter()
                .map(|child| resolved_json(child, sources))
                .collect::<Result<_, _>>()?,
        );
    }
    Ok(value)
}

fn item_json(item: &Item, sources: &crate::Sources) -> Result<Json, serde_json::Error> {
    let mut value = crate::json::metadata(item, sources)?;
    value["kind"] = item
        .ctor()
        .name(sources.definitions())
        .map_err(<serde_json::Error as serde::ser::Error>::custom)?
        .as_ref()
        .into();
    value["start"] = u32::from(item.span.start()).into();
    value["end"] = u32::from(item.span.end()).into();
    value["attrs"] = crate::json::value(&crate::Value::Dict(item.attrs.clone()));
    let mut label = String::new();
    for (key, value) in item.fields().iter() {
        label.push_str(&format!(" :{key} {value}"));
    }
    for (key, value) in item.attrs.iter() {
        label.push_str(&format!(" @{key} {value}"));
    }
    if !label.is_empty() {
        value["label"] = label.into();
    }
    if !item.children().is_empty() {
        value["children"] = Json::Array(
            item.children()
                .iter()
                .map(|child| item_json(child, sources))
                .collect::<Result<_, _>>()?,
        );
    }
    Ok(value)
}
