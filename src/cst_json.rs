use rowan::NodeOrToken;

use notist_core::expr::{BodyFlavor, CallArg, Expr, RExpr};
use notist_syntax::code::ast as code_ast;
use notist_syntax::markup::ast;
use notist_syntax::tree::{SyntaxKind, SyntaxNode, SyntaxToken};

use crate::item::Item;

/// Module inspection keeps the recovered CST available even on semantic errors.
pub fn module_json(src: &str, analysis: &crate::CodeAnalysis) -> String {
    let parse = notist_syntax::code::parse_module(src, &notist_syntax::markup::MarkupContentParser);
    let mut out = String::from("{\"tree\":");
    write_element(&mut out, NodeOrToken::Node(parse.syntax()));
    if let Some(module) = code_ast::Module::cast(parse.syntax()) {
        out.push_str(",\"ast\":");
        out.push_str(&module_ast(&module).to_string());
    }
    match &analysis.definitions {
        Some(module) => {
            out.push_str(",\"functions\":[");
            for (i, function) in module.functions.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&format!(
                    "{{\"function\":{},\"start\":{},\"end\":{},\"signature\":{},\"children\":{},\"level\":{},\"parameters\":[",
                    escape(&function.id.to_string()),
                    u32::from(function.span.start()),
                    u32::from(function.span.end()),
                    escape(&src[usize::from(function.span.start())..usize::from(function.span.end())]),
                    escape(&format!("{:?}", function.children)),
                    function.returns.level().map(|level|escape(if level==crate::builtins::Level::Block{"block"}else{"inline"})).unwrap_or_else(||"null".into()),
                ));
                for (j, parameter) in function.parameters.iter().enumerate() {
                    if j > 0 {
                        out.push(',');
                    }
                    let mode = match parameter.mode {
                        crate::ParameterMode::Required => "required",
                        crate::ParameterMode::Optional => "optional",
                        crate::ParameterMode::Default(_) => "default",
                    };
                    out.push_str(&format!(
                        "{{\"name\":{},\"type\":{},\"mode\":{}",
                        escape(&parameter.name),
                        type_json(&parameter.ty),
                        escape(mode)
                    ));
                    if let crate::ParameterMode::Default(value) = &parameter.mode {
                        out.push_str(",\"default\":");
                        crate::json::write_value(&mut out, value);
                        out.push_str(",\"typed_default\":");
                        out.push_str(&notist_html::components::value_json(value).to_string());
                    }
                    out.push('}');
                }
                out.push(']');
                out.push_str(&format!(",\"code\":{},\"returns\":{},\"children_type\":{},\"comptime\":{}",function.code,type_json(&function.returns),function.children_type.as_ref().map(type_json).unwrap_or_default(),serde_json::json!(function.comptime.iter().map(|p|serde_json::json!({"name":p.name,"type":type_json(&p.ty),"default":p.default.as_ref().map(|v|v.key())})).collect::<Vec<_>>())));
                out.push('}');
            }
            out.push_str("],\"diagnostics\":");
            write_diagnostics(&mut out, &analysis.diagnostics);
            out.push('}');
        }
        None => {
            out.push_str(",\"diagnostics\":");
            write_diagnostics(&mut out, &analysis.diagnostics);
            out.push('}');
        }
    }
    out
}

fn module_ast(module: &code_ast::Module) -> serde_json::Value {
    let functions: Vec<_> = module
        .functions()
        .map(|function| {
            let mut children = function
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
                .collect::<Vec<_>>();
            for parameter in function.comptime_parameters() {
                children.push(ast_node(
                    "ComptimeParameter",
                    parameter.syntax(),
                    Vec::new(),
                ));
            }
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
        })
        .collect();
    let mut children: Vec<_> = module
        .imports()
        .map(|i| ast_node("ImportDecl", i.syntax(), Vec::new()))
        .collect();
    children.extend(functions);
    children.sort_by_key(|node| node["start"].as_u64());
    let mut tree = ast_node("Module", module.syntax(), children);
    tree.as_object_mut().unwrap().remove("label");
    tree
}

fn ast_node(
    kind: &str,
    syntax: &SyntaxNode,
    children: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({"kind":kind, "start":u32::from(syntax.text_range().start()), "end":u32::from(syntax.text_range().end()), "label":syntax.text().to_string(), "children":children})
}

fn type_json(ty: &crate::Type) -> serde_json::Value {
    use crate::Type;
    use serde_json::json;
    match ty {
        Type::Type => json!({"kind":"Type"}),
        Type::InlineContent => json!({"kind":"InlineContent"}),
        Type::BlockContent => json!({"kind":"BlockContent"}),
        Type::Parameter(name) => json!({"kind":"Parameter","name":name}),
        Type::Unit => json!({"kind":"Unit"}),
        Type::Bool => json!({"kind":"Bool"}),
        Type::Int => json!({"kind":"Int"}),
        Type::Float => json!({"kind":"Float"}),
        Type::String => json!({"kind":"String"}),
        Type::Dict => json!({"kind":"Dict"}),
        Type::Array(element) => {
            json!({"kind":"Array", "element":element.as_ref().map(|element| type_json(element))})
        }
        Type::Option(element) => json!({"kind":"Option", "element":type_json(element)}),
    }
}

/// Serialize already computed results; never reruns source processing.
pub fn inspection_json(
    analysis: &crate::Analysis,
    inspection: Option<&notist_pipeline::Inspection>,
) -> String {
    let mut out = String::from("{");
    if let Some(inspection) = inspection {
        if let Some(syntax) = inspection.syntax.as_deref().and_then(|syntax| {
            syntax
                .as_any()
                .downcast_ref::<notist_syntax::tree::SyntaxNode>()
        }) {
            out.push_str("\"tree\":");
            write_element(&mut out, NodeOrToken::Node(syntax.clone()));
            if let Some(document) = ast::Document::cast(syntax.clone()) {
                out.push_str(",\"ast\":");
                write_ast(&mut out, &document);
            }
            out.push(',');
        }
        out.push_str("\"ir1\":");
        write_forest(
            &mut out,
            analysis.root().span,
            &inspection.lowered,
            write_expr,
        );
        out.push_str(",\"ir2\":");
        write_forest(
            &mut out,
            analysis.root().span,
            &inspection.shaped,
            write_resolved,
        );
        out.push(',');
    }
    out.push_str("\"core\":");
    write_item(&mut out, analysis.root());
    out.push_str(",\"diagnostics\":");
    write_diagnostics(&mut out, analysis.diagnostics());
    out.push('}');
    out
}

fn write_diagnostics(out: &mut String, diagnostics: &[crate::Diagnostic]) {
    out.push('[');
    for (i, diagnostic) in diagnostics.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"source\":{},\"start\":{},\"end\":{},\"phase\":{},\"message\":{}}}",
            diagnostic
                .source
                .as_ref()
                .map(|s| escape(s))
                .unwrap_or_else(|| "null".into()),
            u32::from(diagnostic.span.start()),
            u32::from(diagnostic.span.end()),
            escape(&diagnostic.phase.to_string()),
            escape(&diagnostic.message)
        ));
    }
    out.push(']');
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
                        | SyntaxKind::ImportDecl
                        | SyntaxKind::List
                        | SyntaxKind::Raw
                        | SyntaxKind::Divider
                        | SyntaxKind::Table
                        | SyntaxKind::Annotation
                        | SyntaxKind::BlockAnnotation
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
                    SyntaxKind::Annotation | SyntaxKind::BlockAnnotation => {
                        let annotation = code_ast::Annotation::cast(node.clone()).unwrap();
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
                    SyntaxKind::Table | SyntaxKind::ImportDecl => {
                        write_element(out, NodeOrToken::Node(node.clone()))
                    }
                    SyntaxKind::Raw | SyntaxKind::Divider => {
                        let range = node.text_range();
                        out.push_str(&format!(
                            "{{\"kind\":\"{:?}\",\"start\":{},\"end\":{}}}",
                            node.kind(),
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
/// Paragraphs are classified only after Resolve knows the content categories.
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
    out.push_str(&format!(
        "{{\"kind\":\"Inline\",\"start\":{start},\"end\":{end},\"children\":["
    ));
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

fn write_forest<T>(
    out: &mut String,
    range: rowan::TextRange,
    exprs: &[T],
    write: fn(&mut String, &T),
) {
    out.push_str(&format!(
        "{{\"kind\":\"Document\",\"start\":{},\"end\":{},\"children\":[",
        u32::from(range.start()),
        u32::from(range.end()),
    ));
    for (i, expr) in exprs.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write(out, expr);
    }
    out.push_str("]}");
}

fn write_expr(out: &mut String, expr: &Expr) {
    let span = expr.span();
    let (start, end) = (u32::from(span.start()), u32::from(span.end()));
    match expr {
        Expr::Fragment { children, .. } => {
            out.push_str(&format!(
                "{{\"kind\":\"Fragment\",\"start\":{start},\"end\":{end},\"children\":["
            ));
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_expr(out, child);
            }
            out.push_str("]}");
        }
        Expr::Literal(value, _) => {
            out.push_str(&format!(
                "{{\"kind\":\"Literal\",\"start\":{start},\"end\":{end},\"label\":{}}}",
                escape(&value.to_string())
            ));
        }
        Expr::Member { base, name, .. } => {
            out.push_str(&format!(
                "{{\"kind\":\"Member\",\"start\":{start},\"end\":{end},\"label\":{},\"children\":[",
                escape(name)
            ));
            write_expr(out, base);
            out.push_str("]}");
        }
        Expr::Ref { name, .. } => {
            out.push_str(&format!(
                "{{\"kind\":\"Ref\",\"start\":{start},\"end\":{end},\"label\":{}}}",
                escape(name)
            ));
        }
        Expr::Call {
            callee,
            args,
            children,
            body,
            attrs,
            ..
        } => {
            let mut label = expr
                .callee_path()
                .as_deref()
                .unwrap_or("<expression>")
                .to_string();
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
            {
                out.push_str(",\"children\":[");
                let callee_span = callee.span();
                out.push_str(&format!(
                    "{{\"kind\":\"Callee\",\"start\":{},\"end\":{},\"children\":[",
                    u32::from(callee_span.start()),
                    u32::from(callee_span.end()),
                ));
                write_expr(out, callee);
                out.push_str("]}");
                for arg in args {
                    out.push(',');
                    let span = arg.value().span();
                    out.push_str(&format!(
                        "{{\"kind\":\"Arg\",\"start\":{},\"end\":{}",
                        u32::from(span.start()),
                        u32::from(span.end()),
                    ));
                    if let CallArg::Named { name, .. } = arg {
                        out.push_str(&format!(",\"label\":{}", escape(name)));
                    }
                    out.push_str(",\"children\":[");
                    write_expr(out, arg.value());
                    out.push(']');
                    out.push('}');
                }
                for child in children {
                    out.push(',');
                    write_expr(out, child);
                }
                out.push(']');
            }
            out.push('}');
        }
        other => {
            out.push_str(&format!(
                "{{\"kind\":\"Expression\",\"start\":{start},\"end\":{end},\"label\":{}}}",
                escape(&format!("{other:?}"))
            ));
        }
    }
}

fn write_resolved(out: &mut String, expr: &RExpr) {
    let mut label = expr.ctor.to_string();
    for (key, value) in expr.fields.iter() {
        label.push_str(&format!(" :{key} {value}"));
    }
    for (key, value) in expr.attrs.iter() {
        label.push_str(&format!(" @{key} {value}"));
    }
    match expr.body {
        BodyFlavor::Inline => label.push_str(" [inline]"),
        BodyFlavor::Block => label.push_str(" [block]"),
        BodyFlavor::None => {}
    }
    out.push_str(&format!(
        "{{\"kind\":\"Node\",\"start\":{},\"end\":{},\"label\":{}",
        u32::from(expr.span.start()),
        u32::from(expr.span.end()),
        escape(&label),
    ));
    if !expr.children.is_empty() {
        out.push_str(",\"children\":[");
        for (index, child) in expr.children.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            write_resolved(out, child);
        }
        out.push(']');
    }
    out.push('}');
}

fn write_item(out: &mut String, item: &Item) {
    out.push_str(&format!(
        "{{\"kind\":\"{}\",\"start\":{},\"end\":{}",
        item.ctor.name(),
        u32::from(item.span.start()),
        u32::from(item.span.end()),
    ));
    out.push(',');
    // Keep the debug tree's typed children, while exposing all semantic fields.
    crate::json::write_metadata(out, item);
    out.push_str(",\"attrs\":");
    crate::json::write_value(out, &crate::Value::Dict(item.attrs.clone()));
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
