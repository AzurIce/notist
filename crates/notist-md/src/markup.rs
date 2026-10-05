//! Rushdown parsers for explicit Notist constructs. Call bodies stay Markdown;
//! only headers and annotation payloads go through the Notist syntax parser.
use std::{cell::RefCell, fmt};

use notist_core::diag::{Diagnostic, Phase};
use notist_core::expr::{BodyFlavor, Expr};
use notist_core::item::Dict;
use notist_syntax::ast::{Annotation, CodeCall};
use notist_syntax::lexer::lex;
use notist_syntax::syntax::SyntaxKind;
use rowan::{TextRange, TextSize};
use rushdown::ast::{Arena, KindData, NodeKind, NodeRef, NodeType, PrettyPrint};
use rushdown::parser::{BlockParser, Context, InlineParser, State};
use rushdown::text::{BasicReader, BlockReader, Reader};

/// Offset map for lines whose container prefixes were removed by rushdown.
#[derive(Debug, Default)]
struct Source {
    text: String,
    offsets: Vec<usize>,
}

impl Source {
    fn append(&mut self, text: &str, start: usize) {
        self.text.push_str(text);
        self.offsets.extend(start..start + text.len());
    }

    fn range(&self, range: TextRange) -> TextRange {
        let start = usize::from(range.start());
        let end = usize::from(range.end());
        let point = |i: usize| {
            self.offsets
                .get(i)
                .copied()
                .unwrap_or_else(|| self.offsets.last().map_or(0, |last| last + 1))
        };
        let end = if end > start {
            point(end - 1) + 1
        } else {
            point(start)
        };
        TextRange::new((point(start) as u32).into(), (end as u32).into())
    }

    fn remap(&self, expr: &mut Expr) {
        match expr {
            Expr::Literal(_, span) => *span = self.range(*span),
            Expr::Call {
                args,
                children,
                span,
                ..
            } => {
                *span = self.range(*span);
                for expr in args.iter_mut().chain(children.iter_mut()) {
                    self.remap(expr);
                }
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct Markup {
    source: RefCell<Source>,
    block: bool,
}

impl NodeKind for Markup {
    fn typ(&self) -> NodeType {
        if self.block {
            NodeType::LeafBlock
        } else {
            NodeType::Inline
        }
    }
    fn kind_name(&self) -> &'static str {
        "NotistMarkup"
    }
}
impl PrettyPrint for Markup {
    fn pretty_print(&self, w: &mut dyn fmt::Write, _: &str, _: usize) -> fmt::Result {
        write!(w, "{:?}", self.source.borrow().text)
    }
}

struct Construct {
    header: usize,
    end: usize,
    body: Option<(usize, usize)>,
    annotation: bool,
}

enum Scan {
    Complete(Construct),
    Incomplete,
    Text,
}

/// Literal tokens make strings/comments opaque when balancing header parens.
fn scan(src: &str) -> Scan {
    let tokens = lex(src);
    let annotation = tokens.kind(0) == Some(SyntaxKind::At);
    let mut i = 1;
    if annotation {
        if tokens.kind(i) == Some(SyntaxKind::Bang) {
            i += 1;
        }
        if tokens.kind(i) != Some(SyntaxKind::LParen) {
            return Scan::Text;
        }
    } else {
        if tokens.kind(0) != Some(SyntaxKind::Hash) {
            return Scan::Text;
        }
        if tokens.kind(i) == Some(SyntaxKind::Ident) {
            i += 1;
            while tokens.kind(i) == Some(SyntaxKind::ColonColon) {
                if tokens.kind(i + 1) != Some(SyntaxKind::Ident) {
                    return Scan::Text;
                }
                i += 2;
            }
        } else if tokens.kind(i) != Some(SyntaxKind::LBracket) {
            return Scan::Text;
        }
        if !matches!(
            tokens.kind(i),
            Some(SyntaxKind::LParen | SyntaxKind::LBracket)
        ) {
            return Scan::Text;
        }
    }
    if tokens.kind(i) == Some(SyntaxKind::LParen) {
        let mut depth = 0;
        loop {
            match tokens.kind(i) {
                Some(SyntaxKind::LParen) => depth += 1,
                Some(SyntaxKind::RParen) => {
                    depth -= 1;
                    if depth == 0 {
                        i += 1;
                        break;
                    }
                }
                None => return Scan::Incomplete,
                _ => {}
            }
            i += 1;
        }
    }
    let header = usize::from(tokens.offset(i));
    if annotation || tokens.kind(i) != Some(SyntaxKind::LBracket) {
        return Scan::Complete(Construct {
            header,
            end: header,
            body: None,
            annotation,
        });
    }
    // Markdown escapes, code and math are opaque to bracket balancing.
    let bytes = src.as_bytes();
    let mut pos = header + 1;
    let mut depth = 1;
    while pos < bytes.len() {
        if matches!(bytes[pos], b'`' | b'~') {
            let marker = bytes[pos];
            let run = bytes[pos..].iter().take_while(|b| **b == marker).count();
            let line_start = src[..pos].rfind('\n').map_or(0, |i| i + 1);
            if run >= 3 && src[line_start..pos].trim().is_empty() {
                let Some(newline) = src[pos..].find('\n') else {
                    return Scan::Incomplete;
                };
                let mut close = pos + newline + 1;
                let mut closed = false;
                while close < src.len() {
                    let end = close + src[close..].find('\n').unwrap_or(src.len() - close);
                    let line = src[close..end].trim();
                    let count = line.bytes().take_while(|b| *b == marker).count();
                    if count >= run && line[count..].trim().is_empty() {
                        pos = end;
                        closed = true;
                        break;
                    }
                    close = end + 1;
                }
                if !closed {
                    return Scan::Incomplete;
                }
                continue;
            }
        }
        match bytes[pos] {
            b'\\' => {
                pos += 1;
                if pos < bytes.len() {
                    pos += src[pos..].chars().next().unwrap().len_utf8();
                }
                continue;
            }
            b'`' => {
                let count = bytes[pos..].iter().take_while(|b| **b == b'`').count();
                let mut close = pos + count;
                while close < bytes.len() {
                    if bytes[close] == b'`' {
                        let run = bytes[close..].iter().take_while(|b| **b == b'`').count();
                        if run == count {
                            pos = close + count;
                            break;
                        }
                        close += run;
                    } else {
                        close += 1;
                    }
                }
                if close < bytes.len() {
                    continue;
                }
                pos += count;
                continue;
            }
            b'$' if src[pos + 1..]
                .chars()
                .next()
                .is_some_and(|c| !c.is_whitespace()) =>
            {
                let mut close = pos + 1;
                while close < bytes.len() {
                    if bytes[close] == b'\\' {
                        close += 2;
                        continue;
                    }
                    if bytes[close] == b'$'
                        && !src[..close].chars().next_back().unwrap().is_whitespace()
                    {
                        pos = close + 1;
                        break;
                    }
                    close += 1;
                }
                if close < bytes.len() {
                    continue;
                }
            }
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Scan::Complete(Construct {
                        header,
                        end: pos + 1,
                        body: Some((header + 1, pos)),
                        annotation,
                    });
                }
            }
            _ => {}
        }
        pos += 1;
    }
    Scan::Incomplete
}

pub(crate) enum Lowered {
    Annotation {
        attrs: Dict,
        module: bool,
        block: bool,
        span: TextRange,
    },
    Calls(Vec<Expr>),
}

impl Markup {
    pub(crate) fn lower(&self, diagnostics: &mut Vec<Diagnostic>) -> Lowered {
        let source = self.source.borrow();
        let src = &source.text;
        let Scan::Complete(construct) = scan(src) else {
            diagnostics.push(Diagnostic::new(
                Phase::Syntax,
                source.range(TextRange::new(0.into(), (src.len() as u32).into())),
                "unclosed Notist annotation or call",
            ));
            return Lowered::Calls(vec![Expr::text(
                src.to_owned(),
                source.range(TextRange::new(0.into(), (src.len() as u32).into())),
            )]);
        };
        let mut local_diags = Vec::new();
        let mut header = src[..construct.header].to_owned();
        if construct.body.is_some() {
            header.push_str("[]");
        }
        let parse = notist_syntax::parse_document(&header);
        local_diags.extend(
            parse
                .diagnostics
                .iter()
                .map(|d| Diagnostic::new(Phase::Syntax, d.span, d.message.clone())),
        );
        let result = if construct.annotation {
            let annotation = parse
                .syntax()
                .children()
                .find_map(Annotation::cast)
                .unwrap();
            Lowered::Annotation {
                attrs: notist_lowering::annotation_dict(&annotation, &mut local_diags),
                module: annotation.is_module(),
                block: self.block,
                span: source.range(annotation.range()),
            }
        } else {
            let Some(call) = parse.syntax().children().find_map(CodeCall::cast) else {
                diagnostics.extend(local_diags.into_iter().map(|mut diag| {
                    diag.span = source.range(diag.span);
                    diag
                }));
                let span = source.range(TextRange::new(0.into(), (construct.end as u32).into()));
                diagnostics.push(Diagnostic::new(
                    Phase::Syntax,
                    span,
                    "invalid Notist call header",
                ));
                return Lowered::Calls(vec![Expr::text(src.trim_end().to_owned(), span)]);
            };
            let mut expr = notist_lowering::call_header(&call, &mut local_diags);
            if let Expr::Call {
                args,
                children,
                body,
                span,
                ..
            } = &mut expr
            {
                *span = TextRange::new(0.into(), (construct.end as u32).into());
                // Header parsing uses a synthetic empty body. Literal argument
                // ranges follow the full call, as in the native frontend.
                for arg in args {
                    if let Expr::Literal(_, arg_span) = arg {
                        *arg_span = *span;
                    }
                }
                if let Some((start, end)) = construct.body {
                    let content = &src[start..end];
                    let block = content.chars().next().is_some_and(char::is_whitespace)
                        && content.chars().next_back().is_some_and(char::is_whitespace);
                    *body = if block {
                        BodyFlavor::Block
                    } else {
                        BodyFlavor::Inline
                    };
                    let output = crate::compile_body(content, !block, false, false);
                    let forest = output.forest;
                    let mut diags = output.diagnostics;
                    if !block && content.lines().any(|line| line.trim().is_empty()) {
                        diags.push(Diagnostic::new(
                            Phase::Syntax,
                            TextRange::new(0.into(), (content.len() as u32).into()),
                            "inline content cannot contain a blank line",
                        ));
                    }
                    *children = if block {
                        forest
                    } else {
                        forest
                            .into_iter()
                            .flat_map(|expr| match expr {
                                Expr::Call { name, children, .. } if name == "paragraph" => {
                                    children
                                }
                                expr => vec![expr],
                            })
                            .collect()
                    };
                    for child in children.iter_mut() {
                        shift(child, start);
                    }
                    for diag in &mut diags {
                        diag.span += TextSize::new(start as u32);
                    }
                    local_diags.extend(diags);
                }
            }
            source.remap(&mut expr);
            let mut exprs = vec![expr];
            if !src[construct.end..].trim().is_empty() {
                let output = crate::compile_body(&src[construct.end..], false, false, false);
                let mut tail = output.forest;
                let mut diags = output.diagnostics;
                for expr in &mut tail {
                    shift(expr, construct.end);
                    source.remap(expr);
                }
                for diag in &mut diags {
                    diag.span += TextSize::new(construct.end as u32);
                }
                local_diags.extend(diags);
                exprs.extend(tail);
            }
            Lowered::Calls(exprs)
        };
        for mut diag in local_diags {
            diag.span = source.range(diag.span);
            diagnostics.push(diag);
        }
        result
    }
}

fn shift(expr: &mut Expr, offset: usize) {
    match expr {
        Expr::Literal(_, span) => *span += TextSize::new(offset as u32),
        Expr::Call {
            args,
            children,
            span,
            ..
        } => {
            *span += TextSize::new(offset as u32);
            for child in args.iter_mut().chain(children.iter_mut()) {
                shift(child, offset);
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct MarkupInlineParser;
impl InlineParser for MarkupInlineParser {
    fn trigger(&self) -> &[u8] {
        b"@#"
    }
    fn parse(
        &self,
        arena: &mut Arena,
        _: NodeRef,
        reader: &mut BlockReader,
        _: &mut Context,
    ) -> Option<NodeRef> {
        let (line, pos) = reader.position();
        let mut source = Source::default();
        while let Some((text, segment)) = reader.peek_line() {
            source.append(&text, segment.start());
            reader.advance_line();
        }
        reader.set_position(line, pos);
        let end = match scan(&source.text) {
            Scan::Complete(construct) => construct.end,
            Scan::Incomplete => source.text.len(),
            Scan::Text => return None,
        };
        let mut remaining = end;
        while remaining > 0 {
            let (text, _) = reader.peek_line()?;
            let count = remaining.min(text.len());
            reader.advance(count);
            remaining -= count;
        }
        source.text.truncate(end);
        source.offsets.truncate(end);
        Some(arena.new_node(KindData::Extension(Box::new(Markup {
            source: RefCell::new(source),
            block: false,
        }))))
    }
}

#[derive(Debug)]
pub(crate) struct MarkupBlockParser;
impl BlockParser for MarkupBlockParser {
    fn trigger(&self) -> &[u8] {
        b"@#"
    }
    fn open(
        &self,
        arena: &mut Arena,
        _: NodeRef,
        reader: &mut BasicReader,
        ctx: &mut Context,
    ) -> Option<(NodeRef, State)> {
        let (text, segment) = reader.peek_line()?;
        let offset = ctx.block_offset()?;
        let text = &text[offset..];
        match scan(text) {
            Scan::Text => return None,
            Scan::Complete(c) if !text[c.end..].trim().is_empty() => return None,
            _ => {}
        }
        let mut source = Source::default();
        source.append(text, segment.start() + offset);
        reader.advance_to_eol();
        let node = arena.new_node(KindData::Extension(Box::new(Markup {
            source: RefCell::new(source),
            block: true,
        })));
        Some((node, State::NO_CHILDREN))
    }
    fn cont(
        &self,
        arena: &mut Arena,
        node: NodeRef,
        reader: &mut BasicReader,
        _: &mut Context,
    ) -> Option<State> {
        let KindData::Extension(data) = arena[node].kind_data_mut() else {
            unreachable!()
        };
        let markup = data.as_any().downcast_ref::<Markup>().unwrap();
        let mut source = markup.source.borrow_mut();
        if !matches!(scan(&source.text), Scan::Incomplete) {
            return None;
        }
        let (text, segment) = reader.peek_line()?;
        source.append(&text, segment.start());
        reader.advance_to_eol();
        Some(State::NO_CHILDREN)
    }
    fn can_interrupt_paragraph(&self) -> bool {
        true
    }
}

/// Parse an inline-flanked call body as a single Markdown inline sequence.
/// Block markers are ordinary text in this content slot.
#[derive(Debug)]
pub(crate) struct InlineBodyParser;
impl BlockParser for InlineBodyParser {
    fn trigger(&self) -> &[u8] {
        // Rushdown tries character-triggered parsers before free parsers.
        const BYTES: [u8; 256] = {
            let mut bytes = [0; 256];
            let mut i = 0;
            while i < bytes.len() {
                bytes[i] = i as u8;
                i += 1;
            }
            bytes
        };
        &BYTES
    }
    fn open(
        &self,
        arena: &mut Arena,
        _: NodeRef,
        reader: &mut BasicReader,
        _: &mut Context,
    ) -> Option<(NodeRef, State)> {
        let (_, segment) = reader.peek_line()?;
        let node = arena.new_node(rushdown::ast::Paragraph::new());
        rushdown::as_type_data_mut!(arena, node, Block).append_source_line(segment);
        reader.advance_to_eol();
        Some((node, State::NO_CHILDREN))
    }
    fn cont(
        &self,
        arena: &mut Arena,
        node: NodeRef,
        reader: &mut BasicReader,
        _: &mut Context,
    ) -> Option<State> {
        let (_, segment) = reader.peek_line()?;
        rushdown::as_type_data_mut!(arena, node, Block).append_source_line(segment);
        reader.advance_to_eol();
        Some(State::NO_CHILDREN)
    }
    fn can_accept_indented_line(&self) -> bool {
        true
    }
}
