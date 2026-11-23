use crate::ast::{
    BinaryOp, Decl, Expr, ExprKind, Function, Import, ImportKind, ImportMember, Method, Module,
    Param, Program, Span, Type, TypeMatchArm, TypeParam, TypePattern, UnaryOp, WhereExpr,
    WhereExprKind,
};
use crate::content_parser::{
    parse_content_literal, parse_document_source, parse_markdown_document_source,
};
use crate::lexer::{ParseDiagnostic, Token, TokenKind, lex};

const MAX_PARSE_DEPTH: usize = 128;

#[derive(Debug)]
pub struct ParseResult {
    pub program: Program,
    pub diagnostics: Vec<ParseDiagnostic>,
}

/// Parse a single-module `.notc` source string into an AST with source spans.
/// Syntax errors recover at declaration boundaries so subsequent independent
/// declarations are still parsed.
pub fn parse(source: &str) -> ParseResult {
    let (tokens, diagnostics) = lex(source);
    let mut parser = Parser {
        source,
        base_offset: 0,
        tokens,
        pos: 0,
        depth: 0,
        diagnostics,
    };
    let declarations = parser.module();
    ParseResult {
        program: Program {
            root: Module { declarations },
        },
        diagnostics: parser.diagnostics,
    }
}

/// Parse a `.not` document file (top-level Markup + optional `@!(...)` document
/// attributes + leading `#import ...` directives) into a `Program` whose root
/// module exports synthetic `fn __doc__() -> Block`.
pub fn parse_document(source: &str) -> ParseResult {
    let mut diagnostics = Vec::new();
    let root = parse_document_source(source, &mut diagnostics);
    ParseResult {
        program: Program { root },
        diagnostics,
    }
}

/// Parse a `.md` Markdown document file into a `Program` whose root module
/// exports synthetic `fn __doc__() -> Block`.
pub fn parse_markdown_document(source: &str) -> ParseResult {
    let mut diagnostics = Vec::new();
    let root = parse_markdown_document_source(source, &mut diagnostics);
    ParseResult {
        program: Program { root },
        diagnostics,
    }
}

/// Parse a `.not`, `.md`, or `.notc` file based on its file extension.
pub fn parse_file(path: &str, source: &str) -> ParseResult {
    if path.ends_with(".not") {
        parse_document(source)
    } else if path.ends_with(".md") {
        parse_markdown_document(source)
    } else {
        parse(source)
    }
}

/// Parse an embedded `import ...` line from a `.not` document (trailing `;` is optional).
pub(crate) fn parse_embedded_import(
    slice: &str,
    base_offset: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<Import> {
    let (mut tokens, mut lex_diags) = lex(slice);
    offset_tokens(&mut tokens, base_offset as u32);
    offset_diagnostics(&mut lex_diags, base_offset as u32);
    let mut parser = Parser {
        source: slice,
        base_offset,
        tokens,
        pos: 0,
        depth: 0,
        diagnostics: lex_diags,
    };
    let import = parser.import_decl_inner(false).ok();
    diagnostics.extend(parser.diagnostics);
    import
}

/// Parse `#(expr)` embedded inside a `[...]` content literal, returning `(expr, bytes_consumed)`.
pub(crate) fn parse_embedded_expr(
    slice: &str,
    base_offset: usize,
    depth: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<(Expr, usize)> {
    let end = scan_balanced_delims(slice.as_bytes(), 0, b'(', b')')?;
    let sub = &slice[..end];
    let (mut tokens, mut lex_diags) = lex(sub);
    offset_tokens(&mut tokens, base_offset as u32);
    offset_diagnostics(&mut lex_diags, base_offset as u32);
    let mut parser = Parser {
        source: sub,
        base_offset,
        tokens,
        pos: 0,
        depth,
        diagnostics: lex_diags,
    };
    let expr = parser.primary().ok();
    diagnostics.extend(parser.diagnostics);
    expr.map(|e| (e, end))
}

/// Parse `#ident` / `#mod.func<T>(args)[body]` embedded inside a `[...]` content literal.
pub(crate) fn parse_embedded_postfix(
    slice: &str,
    base_offset: usize,
    depth: usize,
    diagnostics: &mut Vec<ParseDiagnostic>,
) -> Option<(Expr, usize)> {
    let end = scan_embedded_postfix_end(slice.as_bytes());
    if end == 0 {
        return None;
    }
    let sub = &slice[..end];
    let (mut tokens, mut lex_diags) = lex(sub);
    offset_tokens(&mut tokens, base_offset as u32);
    offset_diagnostics(&mut lex_diags, base_offset as u32);
    let mut parser = Parser {
        source: sub,
        base_offset,
        tokens,
        pos: 0,
        depth,
        diagnostics: lex_diags,
    };
    let expr = parser.postfix().ok();
    diagnostics.extend(parser.diagnostics);
    expr.map(|e| (e, end))
}

fn offset_tokens(tokens: &mut [Token], offset: u32) {
    if offset == 0 {
        return;
    }
    for tok in tokens {
        tok.span.start += offset;
        tok.span.end += offset;
    }
}

fn offset_diagnostics(diags: &mut [ParseDiagnostic], offset: u32) {
    if offset == 0 {
        return;
    }
    for d in diags {
        d.span.start += offset;
        d.span.end += offset;
    }
}

fn scan_embedded_postfix_end(bytes: &[u8]) -> usize {
    let len = bytes.len();
    let mut i = 0;
    if i >= len || !(bytes[i].is_ascii_alphabetic() || bytes[i] == b'_') {
        return 0;
    }
    i += 1;
    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    loop {
        if i >= len {
            break;
        }
        match bytes[i] {
            b'.' if i + 1 < len && (bytes[i + 1].is_ascii_alphabetic() || bytes[i + 1] == b'_') => {
                i += 2;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
            }
            b'<' => {
                let Some(next) = scan_balanced_delims(bytes, i, b'<', b'>') else {
                    break;
                };
                i = next;
            }
            b'(' => {
                let Some(next) = scan_balanced_delims(bytes, i, b'(', b')') else {
                    break;
                };
                i = next;
            }
            b'[' => {
                let Some(next) = scan_balanced_delims(bytes, i, b'[', b']') else {
                    break;
                };
                i = next;
            }
            _ => break,
        }
    }
    i
}

fn scan_balanced_delims(bytes: &[u8], start: usize, open: u8, close: u8) -> Option<usize> {
    let len = bytes.len();
    if start >= len || bytes[start] != open {
        return None;
    }
    let mut i = start + 1;
    let mut depth = 1usize;
    while i < len && depth > 0 {
        match bytes[i] {
            b'\\' if i + 1 < len => i += 2,
            b'"' if open != b'[' => {
                i += 1;
                while i < len {
                    if bytes[i] == b'\\' && i + 1 < len {
                        i += 2;
                    } else if bytes[i] == b'"' {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
            }
            b if b == open => {
                depth += 1;
                i += 1;
            }
            b if b == close => {
                depth -= 1;
                i += 1;
            }
            _ => i += 1,
        }
    }
    (depth == 0).then_some(i)
}

struct Parser<'a> {
    source: &'a str,
    base_offset: usize,
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    diagnostics: Vec<ParseDiagnostic>,
}

impl<'a> Parser<'a> {
    fn current(&self) -> Token {
        self.tokens[self.pos]
    }

    fn peek(&self, offset: usize) -> Token {
        let index = (self.pos + offset).min(self.tokens.len() - 1);
        self.tokens[index]
    }

    fn kind(&self) -> TokenKind {
        self.current().kind
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.kind() == kind
    }

    fn advance(&mut self) -> Token {
        let token = self.current();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        token
    }

    fn text(&self, span: Span) -> &'a str {
        let start = span.start as usize - self.base_offset;
        let end = span.end as usize - self.base_offset;
        &self.source[start..end]
    }

    fn error(&mut self, span: Span, message: impl Into<String>) {
        self.diagnostics.push(ParseDiagnostic {
            span,
            message: message.into(),
        });
    }

    fn expect(&mut self, expected: TokenKind) -> Result<Token, ()> {
        let token = self.current();
        if token.kind == expected {
            Ok(self.advance())
        } else {
            self.error(
                token.span,
                format!(
                    "expected {}, found {}",
                    expected.describe(),
                    token.kind.describe()
                ),
            );
            Err(())
        }
    }

    fn enter(&mut self) -> Result<(), ()> {
        if self.depth >= MAX_PARSE_DEPTH {
            self.error(self.current().span, "maximum syntax nesting depth exceeded");
            return Err(());
        }
        self.depth += 1;
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn module(&mut self) -> Vec<Decl> {
        let mut declarations = Vec::new();
        while !self.at(TokenKind::Eof) {
            let start_pos = self.pos;
            if self.at(TokenKind::Import) || self.at(TokenKind::Fn) {
                match self.declaration() {
                    Ok(decl) => declarations.push(decl),
                    Err(()) => self.recover_to_next_decl(start_pos),
                }
            } else {
                let token = self.current();
                self.error(
                    token.span,
                    format!(
                        "expected top-level `import` or `fn` declaration, found {}",
                        token.kind.describe()
                    ),
                );
                self.recover_to_next_decl(start_pos);
            }
        }
        declarations
    }

    /// Synchronize to the next top-level declaration without swallowing it.
    fn recover_to_next_decl(&mut self, start_pos: usize) {
        if self.pos == start_pos && !self.at(TokenKind::Eof) {
            self.advance();
        }
        while !self.at(TokenKind::Eof) {
            if self.at_decl_start() {
                break;
            }
            self.advance();
        }
    }

    fn at_decl_start(&self) -> bool {
        if self.at(TokenKind::Import) {
            return true;
        }
        if !self.at(TokenKind::Fn) {
            return false;
        }
        match self.peek(1).kind {
            TokenKind::Ident => true,
            TokenKind::LParen => self.has_dot_after_balanced_parens(self.pos + 1),
            _ => false,
        }
    }

    fn has_dot_after_balanced_parens(&self, open_pos: usize) -> bool {
        let mut depth = 0usize;
        let mut i = open_pos;
        while i < self.tokens.len() {
            match self.tokens[i].kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return i + 1 < self.tokens.len()
                            && self.tokens[i + 1].kind == TokenKind::Dot;
                    }
                }
                TokenKind::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn declaration(&mut self) -> Result<Decl, ()> {
        if self.at(TokenKind::Import) {
            return self.import_decl().map(Decl::Import);
        }
        let fn_tok = self.expect(TokenKind::Fn)?;
        let is_method = match self.kind() {
            TokenKind::Ident => self.peek(1).kind == TokenKind::Dot,
            TokenKind::LParen => true,
            _ => false,
        };
        if is_method {
            self.method_decl(fn_tok.span).map(Decl::Method)
        } else {
            self.function_decl(fn_tok.span).map(Decl::Function)
        }
    }

    fn import_decl(&mut self) -> Result<Import, ()> {
        self.import_decl_inner(true)
    }

    fn import_decl_inner(&mut self, require_semi: bool) -> Result<Import, ()> {
        let import_tok = self.expect(TokenKind::Import)?;
        let path_tok = self.expect(TokenKind::String)?;
        let path = decode_string(self.text(path_tok.span)).map_err(|msg| {
            self.error(path_tok.span, msg);
        })?;
        let kind = if self.at(TokenKind::As) {
            self.advance();
            let alias_tok = self.expect(TokenKind::Ident)?;
            ImportKind::Alias {
                alias: self.text(alias_tok.span).to_owned(),
                span: alias_tok.span,
            }
        } else if self.at(TokenKind::Colon) {
            self.advance();
            if self.at(TokenKind::Star) {
                let star_tok = self.advance();
                ImportKind::Wildcard {
                    span: star_tok.span,
                }
            } else {
                let mut members = Vec::new();
                loop {
                    let name_tok = self.expect(TokenKind::Ident)?;
                    let name = self.text(name_tok.span).to_owned();
                    let (alias, alias_span) = if self.at(TokenKind::As) {
                        self.advance();
                        let alias_tok = self.expect(TokenKind::Ident)?;
                        (self.text(alias_tok.span).to_owned(), alias_tok.span)
                    } else {
                        (name.clone(), name_tok.span)
                    };
                    members.push(ImportMember {
                        name,
                        name_span: name_tok.span,
                        alias,
                        alias_span,
                    });
                    if self.at(TokenKind::Comma) {
                        self.advance();
                        if self.at(TokenKind::Semi) || self.at(TokenKind::Eof) {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                ImportKind::Members(members)
            }
        } else {
            ImportKind::Default
        };
        let end_span = if require_semi {
            self.expect(TokenKind::Semi)?.span
        } else if self.at(TokenKind::Semi) {
            self.advance().span
        } else {
            self.tokens[self.pos.saturating_sub(1)].span
        };
        Ok(Import {
            path,
            path_span: path_tok.span,
            kind,
            span: import_tok.span.merge(end_span),
        })
    }

    fn function_decl(&mut self, start: Span) -> Result<Function, ()> {
        let name_tok = self.expect(TokenKind::Ident)?;
        let name = self.text(name_tok.span).to_owned();
        let type_params = if self.at(TokenKind::Lt) {
            self.type_param_list()?
        } else {
            Vec::new()
        };
        self.expect(TokenKind::LParen)?;
        let parameters = self.param_list()?;
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let returns = self.parse_type()?;
        let where_clause = if self.at(TokenKind::Where) {
            self.advance();
            Some(self.parse_where_expr()?)
        } else {
            None
        };
        let (body, end_span) = if self.at(TokenKind::Semi) {
            let semi = self.advance();
            (None, semi.span)
        } else {
            let (body, body_span) = self.block()?;
            (Some(body), body_span)
        };
        let span = start.merge(end_span);
        Ok(Function {
            name,
            name_span: name_tok.span,
            type_params,
            parameters,
            returns,
            where_clause,
            body,
            span,
        })
    }

    fn type_param_list(&mut self) -> Result<Vec<TypeParam>, ()> {
        self.expect(TokenKind::Lt)?;
        let mut params = Vec::new();
        while !self.at(TokenKind::Gt) && !self.at(TokenKind::Eof) {
            let name_tok = self.expect(TokenKind::Ident)?;
            let name = self.text(name_tok.span).to_owned();
            self.expect(TokenKind::Colon)?;
            let kind_tok = self.expect(TokenKind::Ident)?;
            if self.text(kind_tok.span) != "Type" {
                self.error(
                    kind_tok.span,
                    format!(
                        "expected `Type` for compile-time type parameter `{name}`, found `{}`",
                        self.text(kind_tok.span)
                    ),
                );
                return Err(());
            }
            params.push(TypeParam {
                name,
                span: name_tok.span,
            });
            if self.at(TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(TokenKind::Gt)?;
        Ok(params)
    }

    fn parse_where_expr(&mut self) -> Result<WhereExpr, ()> {
        self.enter()?;
        let result = self.where_or();
        self.leave();
        result
    }

    fn where_or(&mut self) -> Result<WhereExpr, ()> {
        let mut left = self.where_and()?;
        while self.at(TokenKind::OrOr) {
            self.advance();
            let right = self.where_and()?;
            let span = left.span.merge(right.span);
            left = WhereExpr::new(WhereExprKind::Or(Box::new(left), Box::new(right)), span);
        }
        Ok(left)
    }

    fn where_and(&mut self) -> Result<WhereExpr, ()> {
        let mut left = self.where_atom()?;
        while self.at(TokenKind::AndAnd) {
            self.advance();
            let right = self.where_atom()?;
            let span = left.span.merge(right.span);
            left = WhereExpr::new(WhereExprKind::And(Box::new(left), Box::new(right)), span);
        }
        Ok(left)
    }

    fn where_atom(&mut self) -> Result<WhereExpr, ()> {
        match self.kind() {
            TokenKind::Bang => {
                let tok = self.advance();
                let operand = self.where_atom()?;
                let span = tok.span.merge(operand.span);
                Ok(WhereExpr::new(WhereExprKind::Not(Box::new(operand)), span))
            }
            TokenKind::True => {
                let tok = self.advance();
                Ok(WhereExpr::new(WhereExprKind::Bool(true), tok.span))
            }
            TokenKind::False => {
                let tok = self.advance();
                Ok(WhereExpr::new(WhereExprKind::Bool(false), tok.span))
            }
            TokenKind::LParen => {
                let open = self.advance();
                // Distinguish parenthesized where-expr `(A == Int || A == Float)`
                // from parenthesized type `(fn(Int) -> Int) == T`.
                let saved_pos = self.pos;
                let saved_diags = self.diagnostics.len();
                if let Ok(mut inner) = self.where_or()
                    && self.at(TokenKind::RParen)
                {
                    let close = self.advance();
                    inner.span = open.span.merge(close.span);
                    return Ok(inner);
                }
                self.pos = saved_pos - 1;
                self.diagnostics.truncate(saved_diags);
                self.where_type_cmp()
            }
            _ => self.where_type_cmp(),
        }
    }

    fn where_type_cmp(&mut self) -> Result<WhereExpr, ()> {
        let start = self.current().span;
        let left = self.parse_type()?;
        let is_eq = match self.kind() {
            TokenKind::EqEq => {
                self.advance();
                true
            }
            TokenKind::BangEq => {
                self.advance();
                false
            }
            _ => {
                let tok = self.current();
                self.error(
                    tok.span,
                    format!(
                        "expected `==` or `!=` in `where` constraint, found {}",
                        tok.kind.describe()
                    ),
                );
                return Err(());
            }
        };
        let right = self.parse_type()?;
        let end = self.tokens[self.pos.saturating_sub(1)].span;
        let kind = if is_eq {
            WhereExprKind::TypeEq(left, right)
        } else {
            WhereExprKind::TypeNe(left, right)
        };
        Ok(WhereExpr::new(kind, start.merge(end)))
    }

    fn method_decl(&mut self, start: Span) -> Result<Method, ()> {
        let receiver_type = self.parse_type()?;
        self.expect(TokenKind::Dot)?;
        let name_tok = self.expect(TokenKind::Ident)?;
        let name = self.text(name_tok.span).to_owned();
        self.expect(TokenKind::LParen)?;
        let receiver_tok = self.expect(TokenKind::Ident)?;
        let receiver_name = self.text(receiver_tok.span).to_owned();
        let parameters = if self.at(TokenKind::Comma) {
            self.advance();
            self.param_list()?
        } else {
            Vec::new()
        };
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let returns = self.parse_type()?;
        let (body, body_span) = self.block()?;
        let span = start.merge(body_span);
        Ok(Method {
            receiver_type,
            name,
            name_span: name_tok.span,
            receiver_name,
            receiver_span: receiver_tok.span,
            parameters,
            returns,
            body,
            span,
        })
    }

    fn param_list(&mut self) -> Result<Vec<Param>, ()> {
        let mut params = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            let name_tok = self.expect(TokenKind::Ident)?;
            let name = self.text(name_tok.span).to_owned();
            self.expect(TokenKind::Colon)?;
            let ty = self.parse_type()?;
            params.push(Param {
                name,
                ty,
                span: name_tok.span,
            });
            if self.at(TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(params)
    }

    fn parse_type(&mut self) -> Result<Type, ()> {
        self.enter()?;
        let result = self.parse_type_inner();
        self.leave();
        result
    }

    fn parse_type_inner(&mut self) -> Result<Type, ()> {
        match self.kind() {
            TokenKind::Ident => {
                let tok = self.advance();
                match self.text(tok.span) {
                    "Unit" => Ok(Type::Unit),
                    "Bool" => Ok(Type::Bool),
                    "Int" => Ok(Type::Int),
                    "Float" => Ok(Type::Float),
                    "String" => Ok(Type::String),
                    "Inline" => Ok(Type::Inline),
                    "Block" => Ok(Type::Block),
                    "Json" => Ok(Type::Json),
                    "List" => {
                        self.expect(TokenKind::Lt)?;
                        let elem = self.parse_type()?;
                        self.expect(TokenKind::Gt)?;
                        Ok(Type::List(Box::new(elem)))
                    }
                    "Never" => Ok(Type::Never),
                    other => Ok(Type::Param(other.to_owned())),
                }
            }
            TokenKind::Fn => {
                self.advance();
                self.expect(TokenKind::LParen)?;
                let mut params = Vec::new();
                while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
                    params.push(self.parse_type()?);
                    if self.at(TokenKind::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
                self.expect(TokenKind::Arrow)?;
                let returns = self.parse_type()?;
                Ok(Type::Function(params, Box::new(returns)))
            }
            TokenKind::LParen => {
                self.advance();
                let inner = self.parse_type()?;
                self.expect(TokenKind::RParen)?;
                Ok(inner)
            }
            _ => {
                let tok = self.current();
                self.error(
                    tok.span,
                    format!("expected type, found {}", tok.kind.describe()),
                );
                Err(())
            }
        }
    }

    fn block(&mut self) -> Result<(Expr, Span), ()> {
        self.enter()?;
        let result = self.block_inner();
        self.leave();
        result
    }

    fn block_inner(&mut self) -> Result<(Expr, Span), ()> {
        enum Stmt {
            Let(Span, String, Expr),
            Expr(Expr),
        }

        let open = self.expect(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        let mut tail = None;

        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            if self.at(TokenKind::Let) {
                let let_tok = self.advance();
                let name_tok = self.expect(TokenKind::Ident)?;
                let name = self.text(name_tok.span).to_owned();
                self.expect(TokenKind::Eq)?;
                let initializer = self.expression()?;
                self.expect(TokenKind::Semi)?;
                stmts.push(Stmt::Let(let_tok.span, name, initializer));
            } else {
                let expr = self.expression()?;
                if self.at(TokenKind::Semi) {
                    self.advance();
                    stmts.push(Stmt::Expr(expr));
                } else {
                    tail = Some(expr);
                    break;
                }
            }
        }

        let close = self.expect(TokenKind::RBrace)?;
        let block_span = open.span.merge(close.span);
        let mut expr = tail.unwrap_or_else(|| Expr::new(ExprKind::Unit, block_span));
        for stmt in stmts.into_iter().rev() {
            match stmt {
                Stmt::Let(let_span, name, initializer) => {
                    let span = let_span.merge(expr.span);
                    expr = Expr::new(
                        ExprKind::Let(name, Box::new(initializer), Box::new(expr)),
                        span,
                    );
                }
                Stmt::Expr(first) => {
                    let span = first.span.merge(expr.span);
                    expr = Expr::new(ExprKind::Seq(Box::new(first), Box::new(expr)), span);
                }
            }
        }
        Ok((expr, block_span))
    }

    fn expression(&mut self) -> Result<Expr, ()> {
        self.enter()?;
        let result = self.expression_inner();
        self.leave();
        result
    }

    fn expression_inner(&mut self) -> Result<Expr, ()> {
        if self.at(TokenKind::If) {
            self.if_expr()
        } else if self.at(TokenKind::For) {
            self.for_expr()
        } else if self.at(TokenKind::Match) {
            self.type_match_expr()
        } else {
            self.logical_or()
        }
    }

    fn for_expr(&mut self) -> Result<Expr, ()> {
        let for_tok = self.expect(TokenKind::For)?;
        let bind_tok = self.expect(TokenKind::Ident)?;
        let binding = self.text(bind_tok.span).to_owned();
        self.expect(TokenKind::In)?;
        let iterable = self.expression()?;
        let (body, body_span) = self.block()?;
        Ok(Expr::new(
            ExprKind::For {
                binding,
                binding_span: bind_tok.span,
                iterable: Box::new(iterable),
                body: Box::new(body),
            },
            for_tok.span.merge(body_span),
        ))
    }

    fn type_match_expr(&mut self) -> Result<Expr, ()> {
        let match_tok = self.expect(TokenKind::Match)?;
        let param_tok = self.expect(TokenKind::Ident)?;
        let param = self.text(param_tok.span).to_owned();
        self.expect(TokenKind::LBrace)?;
        let mut arms = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let arm_start = self.current().span;
            let pattern = if self.at(TokenKind::Ident) && self.text(self.current().span) == "_" {
                self.advance();
                TypePattern::Wildcard
            } else {
                TypePattern::Type(self.parse_type()?)
            };
            self.expect(TokenKind::FatArrow)?;
            let body = self.expression()?;
            let arm_span = arm_start.merge(body.span);
            arms.push(TypeMatchArm {
                pattern,
                body,
                span: arm_span,
            });
            if self.at(TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        let close = self.expect(TokenKind::RBrace)?;
        Ok(Expr::new(
            ExprKind::TypeMatch {
                param,
                param_span: param_tok.span,
                arms,
            },
            match_tok.span.merge(close.span),
        ))
    }

    fn if_expr(&mut self) -> Result<Expr, ()> {
        let if_tok = self.expect(TokenKind::If)?;
        let condition = self.expression()?;
        let (yes, _) = self.block()?;
        self.expect(TokenKind::Else)?;
        let (no, end_span) = if self.at(TokenKind::If) {
            let expr = self.if_expr()?;
            let span = expr.span;
            (expr, span)
        } else {
            self.block()?
        };
        let span = if_tok.span.merge(end_span);
        Ok(Expr::new(
            ExprKind::If(Box::new(condition), Box::new(yes), Box::new(no)),
            span,
        ))
    }

    fn logical_or(&mut self) -> Result<Expr, ()> {
        let mut left = self.logical_and()?;
        while self.at(TokenKind::OrOr) {
            self.advance();
            let right = self.logical_and()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary(BinaryOp::Or, Box::new(left), Box::new(right)),
                span,
            );
        }
        Ok(left)
    }

    fn logical_and(&mut self) -> Result<Expr, ()> {
        let mut left = self.equality()?;
        while self.at(TokenKind::AndAnd) {
            self.advance();
            let right = self.equality()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary(BinaryOp::And, Box::new(left), Box::new(right)),
                span,
            );
        }
        Ok(left)
    }

    fn equality(&mut self) -> Result<Expr, ()> {
        let mut left = self.compare()?;
        loop {
            let op = match self.kind() {
                TokenKind::EqEq => BinaryOp::Eq,
                TokenKind::BangEq => BinaryOp::Ne,
                _ => break,
            };
            self.advance();
            let right = self.compare()?;
            let span = left.span.merge(right.span);
            left = Expr::new(ExprKind::Binary(op, Box::new(left), Box::new(right)), span);
        }
        Ok(left)
    }

    fn compare(&mut self) -> Result<Expr, ()> {
        let mut left = self.additive()?;
        loop {
            let op = match self.kind() {
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::LtEq => BinaryOp::Le,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::GtEq => BinaryOp::Ge,
                _ => break,
            };
            self.advance();
            let right = self.additive()?;
            let span = left.span.merge(right.span);
            left = Expr::new(ExprKind::Binary(op, Box::new(left), Box::new(right)), span);
        }
        Ok(left)
    }

    fn additive(&mut self) -> Result<Expr, ()> {
        let mut left = self.multiplicative()?;
        loop {
            let op = match self.kind() {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.multiplicative()?;
            let span = left.span.merge(right.span);
            left = Expr::new(ExprKind::Binary(op, Box::new(left), Box::new(right)), span);
        }
        Ok(left)
    }

    fn multiplicative(&mut self) -> Result<Expr, ()> {
        let mut left = self.prefix()?;
        loop {
            let op = match self.kind() {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Rem,
                _ => break,
            };
            self.advance();
            let right = self.prefix()?;
            let span = left.span.merge(right.span);
            left = Expr::new(ExprKind::Binary(op, Box::new(left), Box::new(right)), span);
        }
        Ok(left)
    }

    fn prefix(&mut self) -> Result<Expr, ()> {
        match self.kind() {
            TokenKind::Bang => {
                let tok = self.advance();
                let operand = self.prefix()?;
                let span = tok.span.merge(operand.span);
                Ok(Expr::new(
                    ExprKind::Unary(UnaryOp::Not, Box::new(operand)),
                    span,
                ))
            }
            TokenKind::Minus => {
                let tok = self.advance();
                // Allow `-9223372036854775808` (i64::MIN) when immediately followed by an integer literal
                // and not a postfix call/method on that literal.
                if self.at(TokenKind::Int)
                    && !matches!(self.peek(1).kind, TokenKind::Dot | TokenKind::LParen)
                {
                    let int_tok = self.advance();
                    let raw = self.text(int_tok.span);
                    let span = tok.span.merge(int_tok.span);
                    let value = parse_neg_i64(raw).ok_or_else(|| {
                        self.error(span, format!("integer literal `-{raw}` overflows `Int`"));
                    })?;
                    return Ok(Expr::new(ExprKind::Int(value), span));
                }
                let operand = self.prefix()?;
                let span = tok.span.merge(operand.span);
                Ok(Expr::new(
                    ExprKind::Unary(UnaryOp::Neg, Box::new(operand)),
                    span,
                ))
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<Expr, ()> {
        let mut expr = self.primary()?;
        loop {
            match self.kind() {
                // `<` immediately adjacent to a target starts compile-time type arguments
                // (e.g. `identity<Int>(41)` or `identity<Int>`), while whitespace separates
                // comparison `<` (e.g. `a < b`).
                TokenKind::Lt if self.current().span.start == expr.span.end => {
                    self.advance();
                    let mut type_args = Vec::new();
                    while !self.at(TokenKind::Gt) && !self.at(TokenKind::Eof) {
                        type_args.push(self.parse_type()?);
                        if self.at(TokenKind::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let close = self.expect(TokenKind::Gt)?;
                    let span = expr.span.merge(close.span);
                    expr = Expr::new(ExprKind::Instantiate(Box::new(expr), type_args), span);
                }
                TokenKind::LParen => {
                    self.advance();
                    let args = self.arg_list()?;
                    let close = self.expect(TokenKind::RParen)?;
                    let span = expr.span.merge(close.span);
                    expr = Expr::new(ExprKind::Call(Box::new(expr), args), span);
                }
                // Trailing `[...]` immediately adjacent to a callee or call binds as
                // the next ordinary positional argument (`callout("tip")[ body ]`,
                // `strong[hi]`, `columns[ left ][ right ]`).
                TokenKind::Content if self.current().span.start == expr.span.end => {
                    let content_expr = self.parse_content_token()?;
                    let new_span = expr.span.merge(content_expr.span);
                    match &mut expr.kind {
                        ExprKind::Call(_, args) | ExprKind::MethodCall(_, _, args) => {
                            args.push(content_expr);
                            expr.span = new_span;
                        }
                        _ => {
                            expr = Expr::new(
                                ExprKind::Call(Box::new(expr), vec![content_expr]),
                                new_span,
                            );
                        }
                    }
                }
                TokenKind::Dot => {
                    self.advance();
                    let Ok(name_tok) = self.expect(TokenKind::Ident) else {
                        break;
                    };
                    let member = self.text(name_tok.span).to_owned();
                    if self.at(TokenKind::LParen) {
                        self.advance();
                        let args = self.arg_list()?;
                        let close = self.expect(TokenKind::RParen)?;
                        let span = expr.span.merge(close.span);
                        expr =
                            Expr::new(ExprKind::MethodCall(Box::new(expr), member, args), span);
                    } else if self.at(TokenKind::Content)
                        && self.current().span.start == name_tok.span.end
                    {
                        let content_expr = self.parse_content_token()?;
                        let span = expr.span.merge(content_expr.span);
                        expr = Expr::new(
                            ExprKind::MethodCall(Box::new(expr), member, vec![content_expr]),
                            span,
                        );
                    } else {
                        let span = expr.span.merge(name_tok.span);
                        expr = Expr::new(ExprKind::Member(Box::new(expr), member), span);
                    }
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn arg_list(&mut self) -> Result<Vec<Expr>, ()> {
        let mut args = Vec::new();
        while !self.at(TokenKind::RParen) && !self.at(TokenKind::Eof) {
            args.push(self.expression()?);
            if self.at(TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(args)
    }

    fn primary(&mut self) -> Result<Expr, ()> {
        let tok = self.current();
        match tok.kind {
            TokenKind::Int => {
                self.advance();
                let raw = self.text(tok.span);
                let value = parse_i64(raw).ok_or_else(|| {
                    self.error(tok.span, format!("integer literal `{raw}` overflows `Int`"));
                })?;
                Ok(Expr::new(ExprKind::Int(value), tok.span))
            }
            TokenKind::Float => {
                self.advance();
                let raw = self.text(tok.span);
                let value = parse_f64(raw).ok_or_else(|| {
                    self.error(tok.span, format!("invalid float literal `{raw}`"));
                })?;
                Ok(Expr::new(ExprKind::Float(value), tok.span))
            }
            TokenKind::String => {
                self.advance();
                let raw = self.text(tok.span);
                let value = decode_string(raw).map_err(|msg| {
                    self.error(tok.span, msg);
                })?;
                Ok(Expr::new(ExprKind::String(value), tok.span))
            }
            TokenKind::Content => self.parse_content_token(),
            TokenKind::HashLBracket => {
                self.advance();
                let mut items = Vec::new();
                while !self.at(TokenKind::RBracket) && !self.at(TokenKind::Eof) {
                    items.push(self.expression()?);
                    if self.at(TokenKind::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                let close = self.expect(TokenKind::RBracket)?;
                Ok(Expr::new(
                    ExprKind::ListLiteral(items),
                    tok.span.merge(close.span),
                ))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::new(ExprKind::Bool(true), tok.span))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::new(ExprKind::Bool(false), tok.span))
            }
            TokenKind::Ident => {
                self.advance();
                let name = self.text(tok.span);
                if name == "compile_error" && self.at(TokenKind::LParen) {
                    self.advance();
                    let msg_tok = self.expect(TokenKind::String)?;
                    let msg = decode_string(self.text(msg_tok.span)).map_err(|err| {
                        self.error(msg_tok.span, err);
                    })?;
                    let close = self.expect(TokenKind::RParen)?;
                    return Ok(Expr::new(
                        ExprKind::CompileError(msg),
                        tok.span.merge(close.span),
                    ));
                }
                Ok(Expr::new(ExprKind::Name(name.to_owned()), tok.span))
            }
            TokenKind::LBrace => self.block().map(|(expr, _)| expr),
            TokenKind::LParen => {
                self.advance();
                if self.at(TokenKind::RParen) {
                    let close = self.advance();
                    return Ok(Expr::new(ExprKind::Unit, tok.span.merge(close.span)));
                }
                let mut inner = self.expression()?;
                let close = self.expect(TokenKind::RParen)?;
                inner.span = tok.span.merge(close.span);
                Ok(inner)
            }
            _ => {
                self.error(
                    tok.span,
                    format!("expected expression, found {}", tok.kind.describe()),
                );
                Err(())
            }
        }
    }

    fn parse_content_token(&mut self) -> Result<Expr, ()> {
        self.enter()?;
        let tok = self.expect(TokenKind::Content)?;
        let raw = self.text(tok.span);
        let expr = parse_content_literal(raw, tok.span, self.depth, &mut self.diagnostics);
        self.leave();
        Ok(expr)
    }
}

fn parse_u64(raw: &str) -> Option<u64> {
    let mut acc = 0u64;
    for byte in raw.bytes() {
        if byte == b'_' {
            continue;
        }
        let digit = u64::from(byte - b'0');
        acc = acc.checked_mul(10)?.checked_add(digit)?;
    }
    Some(acc)
}

fn parse_i64(raw: &str) -> Option<i64> {
    if !raw.contains('_') {
        return raw.parse::<i64>().ok();
    }
    i64::try_from(parse_u64(raw)?).ok()
}

fn parse_neg_i64(raw: &str) -> Option<i64> {
    let magnitude = parse_u64(raw)?;
    const MIN_MAGNITUDE: u64 = (i64::MAX as u64) + 1;
    if magnitude == MIN_MAGNITUDE {
        Some(i64::MIN)
    } else if magnitude <= i64::MAX as u64 {
        Some(-(magnitude as i64))
    } else {
        None
    }
}

fn parse_f64(raw: &str) -> Option<f64> {
    let value = if raw.contains('_') {
        let cleaned: String = raw.chars().filter(|&c| c != '_').collect();
        cleaned.parse::<f64>().ok()?
    } else {
        raw.parse::<f64>().ok()?
    };
    value.is_finite().then_some(value)
}

fn decode_string(raw: &str) -> Result<String, &'static str> {
    if let Some(rest) = raw.strip_prefix('r') {
        let hashes = rest.bytes().take_while(|&b| b == b'#').count();
        let after_hashes = &rest[hashes..];
        let body = &after_hashes[..after_hashes.len() - hashes];
        if let Some(inner) = body.strip_prefix("\"\"\"").and_then(|s| s.strip_suffix("\"\"\"")) {
            return strip_multiline_edges(inner).map(str::to_owned);
        }
        let inner = body
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .ok_or("invalid raw string literal")?;
        return Ok(inner.to_owned());
    }

    if let Some(inner) = raw.strip_prefix("\"\"\"").and_then(|s| s.strip_suffix("\"\"\"")) {
        let trimmed = strip_multiline_edges(inner)?;
        return unescape(trimmed);
    }

    let inner = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or("invalid string literal")?;
    unescape(inner)
}

fn strip_multiline_edges(inner: &str) -> Result<&str, &'static str> {
    let after_open = inner
        .strip_prefix("\r\n")
        .or_else(|| inner.strip_prefix('\n'))
        .ok_or("multiline string opening `\"\"\"` must be followed by a newline")?;
    let trimmed = after_open
        .strip_suffix("\r\n")
        .or_else(|| after_open.strip_suffix('\n'))
        .unwrap_or(after_open);
    Ok(trimmed)
}

fn unescape(inner: &str) -> Result<String, &'static str> {
    if !inner.contains('\\') {
        return Ok(inner.to_owned());
    }
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                _ => return Err("invalid escape sequence in string literal"),
            }
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}
