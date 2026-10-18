//! Expression grammar shared by module bodies and interpolation arguments.
use super::{Parser, SyntaxKind as K};

impl Parser<'_, '_> {
    pub(super) fn expression(&mut self, min: u8) {
        self.eat_trivia();
        if self.expression_depth >= 128 {
            self.module_error("expression nesting limit exceeded");
            if self.cur().is_some() {
                self.eat();
            }
            return;
        }
        self.expression_depth += 1;
        let checkpoint = self.state.builder.checkpoint();
        match self.cur() {
            Some(K::IfKeyword) => self.if_expr(),
            Some(K::MatchKeyword) => self.match_expr(),
            Some(K::ComptimeKeyword) => {
                self.state.builder.start_node(K::ComptimeExpr.into());
                self.eat();
                self.expression(0);
                self.state.builder.finish_node();
            }
            Some(K::LBrace) => self.block_expr(),
            Some(K::LBracket) => self.content_literal(),
            Some(K::Bang | K::Minus) => {
                self.state.builder.start_node(K::UnaryExpr.into());
                self.eat();
                self.expression(13);
                self.state.builder.finish_node();
            }
            Some(K::Ident) => self.path(),
            Some(K::Str | K::Number) => self.eat(),
            Some(K::LParen) => self.expression_group(),
            _ => {
                self.module_error("expected an expression");
                if !matches!(
                    self.cur(),
                    None | Some(
                        K::RBrace
                            | K::RParen
                            | K::RBracket
                            | K::FnKeyword
                            | K::Semicolon
                            | K::Greater
                    )
                ) {
                    self.eat();
                }
                self.expression_depth -= 1;
                return;
            }
        }
        loop {
            self.eat_trivia();
            let call = self.cur() == Some(K::LParen)
                || self.cur() == Some(K::LBracket)
                // In expressions an attached `<` starts compile-time arguments;
                // comparison `<` is separated from its left operand by trivia.
                || self.cur() == Some(K::Less)
                    && self.state.pos > 0
                    && !matches!(self.kind_at(self.state.pos - 1), Some(K::Whitespace | K::Newline | K::LineComment | K::BlockComment));
            if call {
                self.state
                    .builder
                    .start_node_at(checkpoint, K::InvokeExpr.into());
                if self.cur() == Some(K::Less) {
                    self.expression_arguments(K::ComptimeArguments, K::Greater);
                    self.eat_trivia();
                }
                if self.cur() == Some(K::LParen) {
                    self.expression_arguments(K::ArgumentList, K::RParen);
                    self.eat_trivia();
                }
                if self.cur() == Some(K::LBracket) {
                    self.content_literal();
                }
                self.state.builder.finish_node();
                continue;
            }
            if self.cur() == Some(K::Greater) && self.in_comptime_args > 0 {
                break;
            }
            let precedence = match self.cur() {
                Some(K::OrOr) => 1,
                Some(K::AndAnd) => 3,
                Some(K::EqEq | K::NotEq) => 5,
                Some(K::Less | K::LessEq | K::Greater | K::GreaterEq) => 7,
                Some(K::Plus | K::Minus) => 9,
                Some(K::Star | K::Slash | K::Percent) => 11,
                _ => break,
            };
            if precedence < min {
                break;
            }
            self.state
                .builder
                .start_node_at(checkpoint, K::BinaryExpr.into());
            self.eat();
            self.expression(precedence + 1);
            self.state.builder.finish_node();
        }
        self.expression_depth -= 1;
    }

    pub(super) fn expression_arguments(&mut self, kind: K, close: K) {
        let outer = if close == K::RParen {
            Some(std::mem::take(&mut self.in_comptime_args))
        } else {
            None
        };
        self.state.builder.start_node(kind.into());
        self.eat();
        self.eat_trivia();
        while !matches!(self.cur(), None | Some(K::FnKeyword | K::RBrace))
            && self.cur() != Some(close)
        {
            let start = self.state.pos;
            let named =
                matches!(self.cur(), Some(K::Ident | K::Str)) && self.peek(1) == Some(K::Colon);
            if named {
                self.state.builder.start_node(K::Entry.into());
                self.eat();
                self.eat();
            }
            // A comptime argument cannot consume its closing `>` as a comparison.
            if close == K::Greater {
                self.in_comptime_args += 1;
            }
            self.expression(0);
            if close == K::Greater {
                self.in_comptime_args -= 1;
            }
            if named {
                self.state.builder.finish_node();
            }
            self.eat_trivia();
            if self.cur() == Some(K::Comma) {
                self.eat();
                self.eat_trivia();
            } else {
                break;
            }
            if self.state.pos == start {
                break;
            }
        }
        self.module_expect(close, "unclosed argument list");
        self.state.builder.finish_node();
        if let Some(outer) = outer {
            self.in_comptime_args = outer;
        }
    }

    fn expression_group(&mut self) {
        let outer = std::mem::take(&mut self.in_comptime_args);
        let checkpoint = self.state.builder.checkpoint();
        self.eat();
        self.eat_trivia();
        let mut count = 0;
        let mut comma = false;
        let mut dict = false;
        if self.cur() == Some(K::Colon) {
            dict = true;
            self.eat();
        }
        if self.cur() == Some(K::Comma) {
            comma = true;
            self.eat();
            self.eat_trivia();
            if self.cur() != Some(K::RParen) {
                self.module_error("empty array literal must end after its comma");
            }
        }
        while !matches!(
            self.cur(),
            None | Some(K::RParen | K::RBrace | K::FnKeyword)
        ) {
            let start = self.state.pos;
            let named =
                matches!(self.cur(), Some(K::Ident | K::Str)) && self.peek(1) == Some(K::Colon);
            if named {
                dict = true;
                self.state.builder.start_node(K::Entry.into());
                self.eat();
                self.eat();
            }
            self.expression(0);
            count += 1;
            if named {
                self.state.builder.finish_node();
            }
            self.eat_trivia();
            if self.cur() == Some(K::Comma) {
                comma = true;
                self.eat();
                self.eat_trivia();
            } else {
                break;
            }
            if start == self.state.pos {
                break;
            }
        }
        self.module_expect(K::RParen, "unclosed group");
        let kind = if dict {
            K::Dict
        } else if count == 0 && !comma {
            K::Unit
        } else if count == 1 && !comma {
            K::Group
        } else {
            K::Array
        };
        self.state.builder.start_node_at(checkpoint, kind.into());
        self.state.builder.finish_node();
        self.in_comptime_args = outer;
    }

    pub(super) fn block_expr(&mut self) {
        self.state.builder.start_node(K::BlockExpr.into());
        self.eat();
        self.eat_trivia();
        while self.cur() == Some(K::LetKeyword) {
            self.state.builder.start_node(K::LetBinding.into());
            self.eat();
            self.eat_trivia();
            self.module_expect(K::Ident, "expected a binding name");
            self.eat_trivia();
            self.module_expect(K::Eq, "expected `=` in a binding");
            self.expression(0);
            self.eat_trivia();
            self.module_expect(K::Semicolon, "expected `;` after a binding");
            self.state.builder.finish_node();
            self.eat_trivia();
        }
        if self.cur() != Some(K::RBrace) {
            self.expression(0);
        }
        self.eat_trivia();
        self.module_expect(K::RBrace, "expected `}` after block expression");
        self.state.builder.finish_node();
    }

    fn if_expr(&mut self) {
        self.state.builder.start_node(K::IfExpr.into());
        self.eat();
        self.expression(0);
        self.eat_trivia();
        if self.cur() == Some(K::LBrace) {
            self.block_expr();
        } else {
            self.module_error("expected an if branch");
        }
        self.eat_trivia();
        self.module_expect(K::ElseKeyword, "expected `else`");
        self.eat_trivia();
        if self.cur() == Some(K::IfKeyword) {
            self.if_expr();
        } else if self.cur() == Some(K::LBrace) {
            self.block_expr();
        } else {
            self.module_error("expected an else branch");
        }
        self.state.builder.finish_node();
    }

    fn match_expr(&mut self) {
        self.state.builder.start_node(K::MatchExpr.into());
        self.eat();
        self.expression(0);
        self.eat_trivia();
        self.module_expect(K::LBrace, "expected `{` after match subject");
        self.eat_trivia();
        while !matches!(self.cur(), None | Some(K::RBrace | K::FnKeyword)) {
            let start = self.state.pos;
            self.state.builder.start_node(K::MatchArm.into());
            self.expression(0);
            self.eat_trivia();
            self.module_expect(K::FatArrow, "expected `=>`");
            self.expression(0);
            self.state.builder.finish_node();
            self.eat_trivia();
            if self.cur() == Some(K::Comma) {
                self.eat();
                self.eat_trivia();
            } else {
                break;
            }
            if start == self.state.pos {
                break;
            }
        }
        self.module_expect(K::RBrace, "expected `}` after match");
        self.state.builder.finish_node();
    }
}
