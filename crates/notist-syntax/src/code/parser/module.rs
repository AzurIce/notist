//! Code declaration grammar. Code parsing owns its token cursor and literal construction.

use super::{Diagnostic, Parse, Parser, SyntaxKind, TextRange};

impl Parser<'_, '_> {
    pub(super) fn run_module(mut self) -> Parse {
        self.state.builder.start_node(SyntaxKind::Module.into());
        while self.cur().is_some() {
            self.eat_trivia();
            match self.cur() {
                None => break,
                Some(SyntaxKind::FnKeyword) => self.function_decl(),
                _ => {
                    self.module_error("expected a function declaration");
                    self.recover_declaration();
                }
            }
        }
        self.state.builder.finish_node();
        self.state.finish()
    }

    fn function_decl(&mut self) {
        self.state
            .builder
            .start_node(SyntaxKind::FunctionDecl.into());
        self.eat(); // fn
        self.eat_trivia();
        let complete = self.function_signature();
        if !complete {
            self.recover_declaration();
        }
        self.state.builder.finish_node();
    }

    fn function_signature(&mut self) -> bool {
        if !self.module_expect(SyntaxKind::Ident, "expected a function name") {
            return false;
        }
        self.eat_trivia();
        if self.cur() == Some(SyntaxKind::Less) {
            self.comptime_parameters();
            self.eat_trivia();
        }
        if self.cur() == Some(SyntaxKind::LParen) && !self.parameter_list() {
            return false;
        }
        self.eat_trivia();
        if self.cur() == Some(SyntaxKind::LBracket) && !self.children_decl() {
            return false;
        }
        self.eat_trivia();
        if !self.module_expect(SyntaxKind::Arrow, "expected `->` before the return type") {
            return false;
        }
        self.eat_trivia();
        if !self.type_ref(0) {
            return false;
        }
        self.eat_trivia();
        if self.cur() == Some(SyntaxKind::LBrace) {
            self.block_expr();
            true
        } else {
            self.module_expect(SyntaxKind::Semicolon, "expected a function body or `;`")
        }
    }

    fn comptime_parameters(&mut self) {
        self.state
            .builder
            .start_node(SyntaxKind::ComptimeParameters.into());
        self.eat();
        self.eat_trivia();
        while !matches!(
            self.cur(),
            None | Some(SyntaxKind::Greater | SyntaxKind::FnKeyword)
        ) {
            let start = self.state.pos;
            self.parameter();
            self.eat_trivia();
            if self.cur() == Some(SyntaxKind::Comma) {
                self.eat();
                self.eat_trivia();
            } else {
                break;
            }
            if self.state.pos == start {
                break;
            }
        }
        self.module_expect(
            SyntaxKind::Greater,
            "expected `>` after comptime parameters",
        );
        self.state.builder.finish_node();
    }

    fn parameter_list(&mut self) -> bool {
        self.state
            .builder
            .start_node(SyntaxKind::ParameterList.into());
        self.eat(); // (
        self.eat_trivia();
        while self.cur() != Some(SyntaxKind::RParen) {
            if self.declaration_boundary()
                || matches!(self.cur(), Some(SyntaxKind::Arrow | SyntaxKind::LBracket))
            {
                break;
            }
            if self.cur() != Some(SyntaxKind::Ident) {
                self.module_error("expected a parameter name");
                self.recover_parameter();
            } else {
                self.parameter();
            }
            self.eat_trivia();
            match self.cur() {
                Some(SyntaxKind::Comma) => {
                    self.eat();
                    self.eat_trivia();
                }
                Some(SyntaxKind::RParen) => break,
                _ if self.declaration_boundary() => break,
                _ => {
                    self.module_error("expected `,` between parameters");
                    // A name starts the next parameter even if its comma is missing.
                    if self.cur() != Some(SyntaxKind::Ident) {
                        self.recover_parameter();
                        if self.cur() == Some(SyntaxKind::Comma) {
                            self.eat();
                            self.eat_trivia();
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        let closed = self.module_expect(SyntaxKind::RParen, "unclosed parameter list");
        self.state.builder.finish_node();
        closed
    }

    fn parameter(&mut self) {
        self.state.builder.start_node(SyntaxKind::Parameter.into());
        self.eat(); // name
        self.eat_trivia();
        let optional = self.cur() == Some(SyntaxKind::Question);
        if optional {
            self.eat();
            self.eat_trivia();
        }
        let typed = self.module_expect(SyntaxKind::Colon, "expected `:` after the parameter name");
        self.eat_trivia();
        if typed && self.type_ref(0) {
            self.eat_trivia();
            if self.cur() == Some(SyntaxKind::Eq) {
                if optional {
                    self.module_error("an optional parameter cannot also have a default");
                }
                self.default_value();
            }
        } else {
            self.recover_parameter();
        }
        self.state.builder.finish_node();
    }

    fn default_value(&mut self) {
        self.state
            .builder
            .start_node(SyntaxKind::DefaultValue.into());
        self.eat(); // =
        self.eat_trivia();
        if matches!(
            self.cur(),
            Some(
                SyntaxKind::Str
                    | SyntaxKind::Number
                    | SyntaxKind::Ident
                    | SyntaxKind::Minus
                    | SyntaxKind::LParen
            )
        ) {
            self.literal_value();
        } else {
            self.module_error("expected a literal default value");
            self.recover_parameter();
        }
        self.state.builder.finish_node();
    }

    fn children_decl(&mut self) -> bool {
        self.state
            .builder
            .start_node(SyntaxKind::ChildrenDecl.into());
        self.eat(); // [
        self.eat_trivia();
        let named = self.cur() == Some(SyntaxKind::Ident)
            && self.state.lexed.text(self.state.pos) == "children";
        if named {
            self.eat();
        } else {
            self.module_error("expected `children` in the content mount declaration");
        }
        self.eat_trivia();
        let colon = named && self.module_expect(SyntaxKind::Colon, "expected `:` after `children`");
        self.eat_trivia();
        let typed = colon && self.type_ref(0);
        self.eat_trivia();
        let closed = typed
            && self.module_expect(
                SyntaxKind::RBracket,
                "expected `]` after the content mount type",
            );
        self.state.builder.finish_node();
        closed
    }

    fn type_ref(&mut self, depth: usize) -> bool {
        if depth >= 128 {
            self.module_error("type nesting limit exceeded");
            return false;
        }
        if self.path_end(self.state.pos).is_none() {
            self.module_error("expected a type name");
            return false;
        }
        self.state.builder.start_node(SyntaxKind::TypeRef.into());
        self.path();
        let mut complete = true;
        let mut after_name = self.state.pos;
        while matches!(
            self.kind_at(after_name),
            Some(
                SyntaxKind::Whitespace
                    | SyntaxKind::Newline
                    | SyntaxKind::LineComment
                    | SyntaxKind::BlockComment
            )
        ) {
            after_name += 1;
        }
        if self.kind_at(after_name) == Some(SyntaxKind::Less) {
            self.eat_trivia();
            self.eat();
            self.eat_trivia();
            loop {
                if !self.type_ref(depth + 1) {
                    complete = false;
                    break;
                }
                self.eat_trivia();
                if self.cur() != Some(SyntaxKind::Comma) {
                    break;
                }
                self.eat();
                self.eat_trivia();
                if self.cur() == Some(SyntaxKind::Greater) {
                    break;
                }
            }
            complete = self.module_expect(SyntaxKind::Greater, "expected `>` after type arguments")
                && complete;
        }
        self.state.builder.finish_node();
        complete
    }

    pub(super) fn module_expect(&mut self, kind: SyntaxKind, message: &str) -> bool {
        if self.cur() == Some(kind) {
            self.eat();
            true
        } else {
            self.module_error(message);
            false
        }
    }

    pub(super) fn module_error(&mut self, message: &str) {
        let start = self.state.lexed.offset(self.state.pos);
        let end = if self.cur().is_some() {
            self.state.lexed.offset(self.state.pos + 1)
        } else {
            start
        };
        self.state.diagnostics.push(Diagnostic {
            span: TextRange::new(start, end),
            message: message.to_string(),
        });
    }

    fn declaration_boundary(&self) -> bool {
        matches!(
            self.cur(),
            None | Some(SyntaxKind::FnKeyword | SyntaxKind::Semicolon)
        )
    }

    /// The next `fn` is never consumed, even inside a broken signature.
    fn recover_declaration(&mut self) {
        if matches!(self.cur(), None | Some(SyntaxKind::FnKeyword)) {
            return;
        }
        self.state.builder.start_node(SyntaxKind::Error.into());
        while self.cur().is_some() && self.cur() != Some(SyntaxKind::FnKeyword) {
            let end = self.cur() == Some(SyntaxKind::Semicolon);
            self.eat();
            if end {
                break;
            }
        }
        self.state.builder.finish_node();
    }

    fn recover_parameter(&mut self) {
        if self.declaration_boundary()
            || matches!(self.cur(), Some(SyntaxKind::Comma | SyntaxKind::RParen))
        {
            return;
        }
        self.state.builder.start_node(SyntaxKind::Error.into());
        while !self.declaration_boundary()
            && !matches!(self.cur(), Some(SyntaxKind::Comma | SyntaxKind::RParen))
        {
            self.eat();
        }
        self.state.builder.finish_node();
    }
}
