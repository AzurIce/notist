use crate::ast::{Decl, Expr, ExprKind, Function, Method, Module, Param, Program, Span, Type};
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

struct Parser<'a> {
    source: &'a str,
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
        &self.source[span.start as usize..span.end as usize]
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
            if self.at(TokenKind::Fn) {
                match self.declaration() {
                    Ok(decl) => declarations.push(decl),
                    Err(()) => self.recover_to_next_decl(start_pos),
                }
            } else {
                let token = self.current();
                self.error(
                    token.span,
                    format!(
                        "expected top-level `fn` declaration, found {}",
                        token.kind.describe()
                    ),
                );
                self.recover_to_next_decl(start_pos);
            }
        }
        declarations
    }

    /// Synchronize to the next top-level declaration without swallowing it.
    /// In this grammar, `fn` followed by an identifier (`fn name(` or
    /// `fn Type.name(`) or a parenthesized type followed by `.` (`fn (fn(...) -> ...).name(`)
    /// always begins a declaration rather than an inline function type.
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

    fn function_decl(&mut self, start: Span) -> Result<Function, ()> {
        let name_tok = self.expect(TokenKind::Ident)?;
        let name = self.text(name_tok.span).to_owned();
        self.expect(TokenKind::LParen)?;
        let parameters = self.param_list()?;
        self.expect(TokenKind::RParen)?;
        self.expect(TokenKind::Arrow)?;
        let returns = self.parse_type()?;
        let body = self.block()?;
        let span = start.merge(body.span);
        Ok(Function {
            name,
            name_span: name_tok.span,
            parameters,
            returns,
            body,
            span,
        })
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
        let body = self.block()?;
        let span = start.merge(body.span);
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
                    "Int" => Ok(Type::Int),
                    "Bool" => Ok(Type::Bool),
                    other => {
                        self.error(tok.span, format!("unknown type `{other}`"));
                        Err(())
                    }
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
                self.error(tok.span, format!("expected type, found {}", tok.kind.describe()));
                Err(())
            }
        }
    }

    fn block(&mut self) -> Result<Expr, ()> {
        self.enter()?;
        let result = self.block_inner();
        self.leave();
        result
    }

    fn block_inner(&mut self) -> Result<Expr, ()> {
        let open = self.expect(TokenKind::LBrace)?;
        let mut bindings = Vec::new();
        while self.at(TokenKind::Let) {
            let let_tok = self.advance();
            let name_tok = self.expect(TokenKind::Ident)?;
            let name = self.text(name_tok.span).to_owned();
            self.expect(TokenKind::Eq)?;
            let initializer = self.expression()?;
            self.expect(TokenKind::Semi)?;
            bindings.push((let_tok.span, name, initializer));
        }
        let tail = self.expression()?;
        let close = self.expect(TokenKind::RBrace)?;
        let block_span = open.span.merge(close.span);
        let mut expr = tail;
        for (let_span, name, initializer) in bindings.into_iter().rev() {
            let span = let_span.merge(expr.span);
            expr = Expr::new(ExprKind::Let(name, Box::new(initializer), Box::new(expr)), span);
        }
        expr.span = block_span;
        Ok(expr)
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
        } else {
            self.additive()
        }
    }

    fn if_expr(&mut self) -> Result<Expr, ()> {
        let if_tok = self.expect(TokenKind::If)?;
        let condition = self.expression()?;
        let yes = self.block()?;
        self.expect(TokenKind::Else)?;
        let no = if self.at(TokenKind::If) {
            self.if_expr()?
        } else {
            self.block()?
        };
        let span = if_tok.span.merge(no.span);
        Ok(Expr::new(
            ExprKind::If(Box::new(condition), Box::new(yes), Box::new(no)),
            span,
        ))
    }

    fn additive(&mut self) -> Result<Expr, ()> {
        let mut left = self.postfix()?;
        while self.at(TokenKind::Plus) {
            self.advance();
            let right = self.postfix()?;
            let span = left.span.merge(right.span);
            left = Expr::new(ExprKind::Add(Box::new(left), Box::new(right)), span);
        }
        Ok(left)
    }

    fn postfix(&mut self) -> Result<Expr, ()> {
        let mut expr = self.primary()?;
        loop {
            match self.kind() {
                TokenKind::LParen => {
                    self.advance();
                    let args = self.arg_list()?;
                    let close = self.expect(TokenKind::RParen)?;
                    let span = expr.span.merge(close.span);
                    expr = Expr::new(ExprKind::Call(Box::new(expr), args), span);
                }
                TokenKind::Dot => {
                    self.advance();
                    let name_tok = self.expect(TokenKind::Ident)?;
                    let method = self.text(name_tok.span).to_owned();
                    self.expect(TokenKind::LParen)?;
                    let args = self.arg_list()?;
                    let close = self.expect(TokenKind::RParen)?;
                    let span = expr.span.merge(close.span);
                    expr = Expr::new(ExprKind::MethodCall(Box::new(expr), method, args), span);
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
                let name = self.text(tok.span).to_owned();
                Ok(Expr::new(ExprKind::Name(name), tok.span))
            }
            TokenKind::LBrace => self.block(),
            TokenKind::LParen => {
                self.advance();
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
}

fn parse_i64(raw: &str) -> Option<i64> {
    if !raw.contains('_') {
        return raw.parse::<i64>().ok();
    }
    let mut acc = 0i64;
    for byte in raw.bytes() {
        if byte == b'_' {
            continue;
        }
        let digit = i64::from(byte - b'0');
        acc = acc.checked_mul(10)?.checked_add(digit)?;
    }
    Some(acc)
}
