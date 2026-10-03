use notist_core::diag::Diagnostic;
use notist_core::expr::Expr;
use notist_core::item::{Dict, Value};
use rowan::TextRange;
use rushdown::ast::{Arena, KindData, Node, NodeRef};
use rushdown::parser::{gfm_strikethrough, gfm_table, Options, Parser, ParserExtension};
use rushdown::text::BasicReader;

/// The markdown frontend's lowering: rushdown AST → shared Expr IR.
pub fn lower(src: &str) -> (Vec<Expr>, Dict, Vec<Diagnostic>) {
    let parser = Parser::with_extensions(
        Options::default(),
        gfm_table().and(gfm_strikethrough()),
    );
    let (arena, root) = parser.parse(&mut BasicReader::new(src));
    let lowerer = Lowerer { src, arena: &arena };
    let forest = lowerer.children(root);
    (forest, Dict::default(), Vec::new())
}

struct Lowerer<'a> {
    src: &'a str,
    arena: &'a Arena,
}

impl<'a> Lowerer<'a> {
    fn node(&self, node: NodeRef) -> &'a Node {
        self.arena.get(node).unwrap()
    }

    fn text_of(&self, node: NodeRef) -> String {
        let mut cur = Some(node);
        let mut out = String::new();
        while let Some(c) = cur {
            if let KindData::Text(text) = self.node(c).kind_data() {
                out.push_str(text.str(self.src));
            }
            cur = self.node(c).first_child();
        }
        out
    }

    /// A node's span: its `pos` to the next sibling's `pos` (walking up
    /// through parents when there is none), or EOF.
    fn span_of(&self, node: NodeRef) -> TextRange {
        let start = self.node(node).pos().unwrap_or(0) as u32;
        let mut end = self.src.len() as u32;
        let mut cur = Some(node);
        'walk: while let Some(c) = cur {
            if let Some(next) = self.node(c).next_sibling() {
                if let Some(pos) = self.node(next).pos() {
                    end = pos as u32;
                    break 'walk;
                }
            }
            cur = self.node(c).parent();
        }
        TextRange::new(start.into(), end.into())
    }

    fn children(&self, node: NodeRef) -> Vec<Expr> {
        let mut out = Vec::new();
        let mut text_start = None;
        let mut text_end = 0usize;
        let mut text_buf = String::new();
        let flush = |out: &mut Vec<Expr>,
                         text_start: &mut Option<usize>,
                         text_buf: &mut String,
                         text_end: usize| {
            if let Some(start) = text_start.take() {
                if !text_buf.trim().is_empty() {
                    out.push(Expr::text(
                        std::mem::take(text_buf),
                        TextRange::new((start as u32).into(), (text_end as u32).into()),
                    ));
                } else {
                    text_buf.clear();
                }
            }
        };

        let mut child = self.node(node).first_child();
        while let Some(c) = child {
            let n = self.node(c);
            if let KindData::Text(text) = n.kind_data() {
                if text_start.is_none() {
                    text_start = n.pos();
                }
                text_buf.push_str(text.str(self.src));
                text_end = n.pos().unwrap_or(0) + text.str(self.src).len();
            } else {
                flush(&mut out, &mut text_start, &mut text_buf, text_end);
                out.push(self.item(c));
            }
            child = n.next_sibling();
        }
        flush(&mut out, &mut text_start, &mut text_buf, text_end);
        out
    }

    fn item(&self, node: NodeRef) -> Expr {
        let span = self.span_of(node);
        match self.node(node).kind_data() {
            KindData::Paragraph(_) => Expr::call("paragraph", span).with_children(self.children(node)),
            KindData::Heading(h) => Expr::call("heading", span)
                .with_field("level", Value::Int(h.level() as i64))
                .with_children(self.children(node)),
            KindData::ThematicBreak(_) => Expr::call("thematicbreak", span),
            KindData::CodeBlock(b) => {
                let mut expr = Expr::call("raw", span)
                    .with_field("block", Value::Bool(true))
                    .with_field("text", Value::Str(b.value().iter(self.src).collect::<String>()));
                if let Some(info) = b.info() {
                    let lang = info.str(self.src).trim().to_string();
                    if !lang.is_empty() {
                        expr = expr.with_field("lang", Value::Str(lang));
                    }
                }
                expr
            }
            KindData::Blockquote(_) => Expr::call("blockquote", span).with_children(self.children(node)),
            KindData::List(l) => Expr::call("list", span)
                .with_field("ordered", Value::Bool(l.is_ordered()))
                .with_children(self.children(node)),
            KindData::ListItem(_) => Expr::call("item", span).with_children(self.children(node)),
            KindData::HtmlBlock(b) => Expr::call("html", span).with_field("text", Value::Str(b.value().iter(self.src).collect::<String>())),
            KindData::CodeSpan(c) => Expr::call("raw", span).with_field("text", Value::Str(c.str(self.src).into_owned())),
            KindData::Emphasis(_) => Expr::call("emph", span).with_children(self.children(node)),
            KindData::Strong(_) => Expr::call("strong", span).with_children(self.children(node)),
            KindData::Link(l) => Expr::call("link", span)
                .with_field("target", Value::Str(l.destination().str(self.src).to_string()))
                .with_children(self.children(node)),
            KindData::Image(i) => Expr::call("image", span)
                .with_field("src", Value::Str(i.destination().str(self.src).to_string()))
                .with_field("alt", Value::Str(self.text_of(node)))
                .with_children(self.children(node)),
            KindData::RawHtml(b) => Expr::call("html", span).with_field("text", Value::Str(b.value().str(self.src).into_owned())),
            KindData::Table(_) => Expr::call("table", span).with_children(self.children(node)),
            KindData::TableRow(_) => Expr::call("row", span).with_children(self.children(node)),
            KindData::TableCell(_) => Expr::call("cell", span).with_children(self.children(node)),
            KindData::Strikethrough(_) => Expr::call("strike", span).with_children(self.children(node)),
            _ => {
                // TableHeader / TableBody / LinkReferenceDefinition / 其他：子节点透传
                let mut out = Vec::new();
                out.extend(self.children(node));
                Expr::call("group", span).with_children(out)
            }
        }
    }
}
