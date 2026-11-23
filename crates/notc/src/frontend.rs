//! Pluggable document frontend boundary and fine-grained one-way embedded code API.
//!
//! `notc` itself implements only the core `.notc` language and Notist's native
//! `.not` markup syntax (including `[...]` content literals inside `.notc`).
//! Any other markup language (such as Markdown via `notist-markdown`) sits strictly
//! *above* `notc` by implementing [`DocumentFrontend`] and lowering its AST into a
//! standard [`Module`] exporting synthetic `fn __doc__() -> Block`.

use crate::ast::{
    AttrEntry, BlockPiece, DOC_ENTRY, Decl, Expr, ExprKind, Function, Import, Module, Span, Type,
};
use crate::content_parser::{parse_attr_list_at, parse_document_source};
use crate::lexer::ParseDiagnostic;
use crate::parser::{
    ParseResult, parse_embedded_expr, parse_embedded_import, parse_embedded_postfix,
};

/// A pluggable document-level markup frontend that parses source text into a
/// standard `notc::Module` AST (with any `Decl::Import`s plus `fn __doc__() -> Block`).
pub trait DocumentFrontend: Send + Sync {
    /// File extensions (without leading dot, e.g. `["md", "markdown"]`) handled by this frontend.
    fn extensions(&self) -> &[&str];

    /// Parse a document into a `notc::Module` AST with exact source spans.
    fn parse_document(&self, source: &str) -> ParseResult;
}

/// Built-in frontend for native Notist `.not` documents.
#[derive(Debug, Default, Clone, Copy)]
pub struct NotistFrontend;

impl DocumentFrontend for NotistFrontend {
    fn extensions(&self) -> &[&str] {
        &["not"]
    }

    fn parse_document(&self, source: &str) -> ParseResult {
        let mut diagnostics = Vec::new();
        let root = parse_document_source(source, &mut diagnostics);
        ParseResult {
            program: crate::ast::Program { root },
            diagnostics,
        }
    }
}

/// Package a frontend's parsed imports, optional document-level attributes, and
/// top-level block pieces into a standard `notc::Module` exporting `fn __doc__() -> Block`.
pub fn build_document_module(
    mut declarations: Vec<Decl>,
    doc_attrs: Vec<AttrEntry>,
    blocks: Vec<BlockPiece>,
    body_span: Span,
    full_len: u32,
) -> Module {
    let mut body_expr = Expr::new(ExprKind::BlockLiteral(blocks), body_span);
    if !doc_attrs.is_empty() {
        body_expr = Expr::new(
            ExprKind::AttachAttrs(doc_attrs, Box::new(body_expr)),
            Span::new(0, full_len),
        );
    }
    declarations.push(Decl::Function(Function {
        name: DOC_ENTRY.to_owned(),
        name_span: Span::new(0, 0),
        type_params: Vec::new(),
        parameters: Vec::new(),
        returns: Type::Block,
        where_clause: None,
        body: Some(body_expr),
        span: Span::new(0, full_len),
    }));
    Module { declarations }
}

/// Fine-grained one-way parsing hooks exposed to external [`DocumentFrontend`]
/// implementations so they can embed `#import`, `#call(...)`, `#(...)`, and
/// `@(...)` / `@!(...)` annotations without any circular dependency or parser ping-pong.
pub mod embedded {
    use super::*;

    /// Parse an `import ...` directive (without leading `#`; trailing `;` optional).
    pub fn parse_import(
        slice: &str,
        base_offset: usize,
        diagnostics: &mut Vec<ParseDiagnostic>,
    ) -> Option<Import> {
        parse_embedded_import(slice, base_offset, diagnostics)
    }

    /// Parse a postfix expression (`ident`, `ns.fn(...)`, `fn[body]`, etc., without leading `#`).
    /// Returns `(expr, bytes_consumed)`.
    pub fn parse_postfix(
        slice: &str,
        base_offset: usize,
        diagnostics: &mut Vec<ParseDiagnostic>,
    ) -> Option<(Expr, usize)> {
        parse_embedded_postfix(slice, base_offset, 0, diagnostics)
    }

    /// Parse a parenthesized expression `(...)` (without leading `#`).
    /// Returns `(expr, bytes_consumed)`.
    pub fn parse_expr(
        slice: &str,
        base_offset: usize,
        diagnostics: &mut Vec<ParseDiagnostic>,
    ) -> Option<(Expr, usize)> {
        parse_embedded_expr(slice, base_offset, 0, diagnostics)
    }

    /// Parse an element attribute annotation `@(key: val, ...)` starting at `at_pos` inside `text`.
    /// Returns `(entries, end_byte_offset_in_text)`.
    pub fn parse_attr_list(
        text: &str,
        at_pos: usize,
        base_offset: usize,
        diagnostics: &mut Vec<ParseDiagnostic>,
    ) -> Option<(Vec<AttrEntry>, usize)> {
        let bytes = text.as_bytes();
        if at_pos + 1 >= bytes.len() || bytes[at_pos] != b'@' || bytes[at_pos + 1] != b'(' {
            return None;
        }
        parse_attr_list_at(text, at_pos, at_pos + 1, base_offset, diagnostics)
    }

    /// Parse a document attribute annotation `@!(key: val, ...)` starting at `at_pos` inside `text`.
    /// Returns `(entries, end_byte_offset_in_text)`.
    pub fn parse_doc_attr_list(
        text: &str,
        at_pos: usize,
        base_offset: usize,
        diagnostics: &mut Vec<ParseDiagnostic>,
    ) -> Option<(Vec<AttrEntry>, usize)> {
        let bytes = text.as_bytes();
        if at_pos + 2 >= bytes.len()
            || bytes[at_pos] != b'@'
            || bytes[at_pos + 1] != b'!'
            || bytes[at_pos + 2] != b'('
        {
            return None;
        }
        parse_attr_list_at(text, at_pos, at_pos + 2, base_offset, diagnostics)
    }
}
