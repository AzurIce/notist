use crate::{CharacterSetItem, Error, Expr, ExprKind, Grammar, Import, Layer, RangeLimit, Rule};
use std::ops::Range;

const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_DEPTH: usize = 64;
const MAX_NODES: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    Word(String),
    Literal(String),
    Unicode(char, String),
    Quoted,
    Code(String),
    Arguments(String),
    Prose(String),
    Yield,
    Symbol(char),
    Newline,
    Eof,
}

#[derive(Clone, Debug)]
struct Token {
    kind: Kind,
    span: Range<usize>,
}

/// Parse Rust Reference notation with Notist's explicit context extensions.
/// Bare productions default to syntax; `lex` declares character-level input.
/// Code fragments and invocation arguments are preserved without evaluation.
pub fn parse(source: &str) -> Result<Grammar, Error> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(Error::new(
            source,
            0..0,
            "grammar exceeds the 1 MiB source limit",
        ));
    }
    let mut parser = Parser {
        source,
        tokens: lex(source)?,
        pos: 0,
        depth: 0,
        nodes: 0,
        building: false,
        layer: Layer::Syntax,
    };
    let mut grammar = Grammar {
        rules: Vec::new(),
        imports: Vec::new(),
    };
    parser.newlines();
    while !matches!(parser.token().kind, Kind::Eof) {
        let start = parser.token().span.start;
        if parser.take_word("import") {
            let kind = parser.name()?;
            if !["lex", "syntax", "ast", "scanner", "predicate"].contains(&kind.as_str()) {
                return Err(
                    parser.error("expected an import kind: lex, syntax, ast, scanner or predicate")
                );
            }
            let name = parser.name()?;
            let parameters = parser.arguments()?;
            if !parser.take_word("from") {
                return Err(parser.error("expected `from` in import"));
            }
            let from = parser.literal()?;
            let end = parser.previous_end();
            parser.terminator()?;
            grammar.imports.push(Import {
                kind,
                name,
                parameters,
                from,
                span: start..end,
            });
        } else {
            let is_root = if parser.take('@') {
                if !parser.take_word("root") {
                    return Err(parser.error("expected `@root` before a production"));
                }
                parser.newlines();
                true
            } else {
                false
            };
            let layer = if parser.take_word("lex") {
                Layer::Lex
            } else if parser.take_word("ast") {
                Layer::Ast
            } else {
                parser.take_word("syntax");
                Layer::Syntax
            };
            parser.layer = layer;
            parser.building = false;
            let name = parser.name()?;
            if grammar.rules.iter().any(|rule| rule.name == name) {
                return Err(parser.error(format!("duplicate rule `{name}`")));
            }
            let parameters = parser.arguments()?;
            parser.expect('-')?;
            parser.expect('>')?;
            let expression = parser.expression()?;
            let result = if matches!(parser.token().kind, Kind::Yield) {
                parser.pos += 1;
                parser.building = true;
                Some(parser.expression()?)
            } else {
                None
            };
            let end = result.as_ref().unwrap_or(&expression).span.end;
            parser.terminator()?;
            grammar.rules.push(Rule {
                layer,
                name,
                is_root,
                parameters,
                expression,
                result,
                span: start..end,
            });
        }
        parser.newlines();
    }
    if grammar.rules.is_empty() && grammar.imports.is_empty() {
        return Err(parser.error("expected at least one grammar production or import"));
    }
    Ok(grammar)
}

fn lex(source: &str) -> Result<Vec<Token>, Error> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut offset = 0;
    while offset < source.len() {
        let start = offset;
        let ch = source[offset..].chars().next().unwrap();
        offset += ch.len_utf8();
        let kind = match ch {
            ' ' | '\t' => continue,
            '\n' | '\r' => {
                if ch == '\r' && source[offset..].starts_with('\n') {
                    offset += 1;
                }
                Kind::Newline
            }
            '/' if source[offset..].starts_with('/') => {
                offset = line_comment_end(source, offset);
                continue;
            }
            '`' => {
                let text_start = offset;
                while offset < source.len() && !source[offset..].starts_with(['`', '\n', '\r']) {
                    offset += source[offset..].chars().next().unwrap().len_utf8();
                }
                if !source[offset..].starts_with('`') {
                    return Err(Error::new(
                        source,
                        start..offset,
                        "unterminated grammar terminal",
                    ));
                }
                let text = source[text_start..offset].to_owned();
                if text.is_empty() {
                    return Err(Error::new(
                        source,
                        start..offset + 1,
                        "empty terminal; use ε for an empty match",
                    ));
                }
                offset += 1;
                Kind::Literal(text)
            }
            'U' if source[offset..].starts_with('+') => {
                offset += 1;
                let hex_start = offset;
                while source[offset..].starts_with(|c: char| c.is_ascii_hexdigit()) {
                    offset += 1;
                }
                let hex = &source[hex_start..offset];
                let character = if (4..=6).contains(&hex.len()) {
                    u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                } else {
                    None
                };
                let Some(character) = character else {
                    return Err(Error::new(
                        source,
                        start..offset,
                        "expected U+ followed by 4 to 6 hex digits for a Unicode scalar",
                    ));
                };
                Kind::Unicode(character, hex.to_owned())
            }
            '"' | '\'' => {
                offset = quoted_end(source, start, ch)?;
                Kind::Quoted
            }
            '=' if source[offset..].starts_with('>') => {
                offset += 1;
                Kind::Yield
            }
            '{' => {
                offset = code_end(source, start)?;
                Kind::Code(source[start + 1..offset - 1].trim().to_owned())
            }
            '(' if tokens.last().is_some_and(|token| {
                matches!(&token.kind, Kind::Word(name) if name != "within")
                    && token.span.end == start
            }) =>
            {
                offset = code_end(source, start)?;
                Kind::Arguments(source[start + 1..offset - 1].trim().to_owned())
            }
            '<' => {
                while offset < source.len() && !source[offset..].starts_with(['>', '\r', '\n']) {
                    offset += source[offset..].chars().next().unwrap().len_utf8();
                }
                if !source[offset..].starts_with('>') {
                    return Err(Error::new(
                        source,
                        start..offset,
                        "unterminated grammar prose",
                    ));
                }
                let text = source[start + 1..offset].trim().to_owned();
                if text.is_empty() {
                    return Err(Error::new(
                        source,
                        start..offset + 1,
                        "expected grammar prose",
                    ));
                }
                offset += 1;
                Kind::Prose(text)
            }
            c if c.is_alphanumeric() || c == '_' => {
                while source[offset..].starts_with(|c: char| c.is_alphanumeric() || c == '_') {
                    offset += source[offset..].chars().next().unwrap().len_utf8();
                }
                Kind::Word(source[start..offset].to_owned())
            }
            c => Kind::Symbol(c),
        };
        tokens.push(Token {
            kind,
            span: start..offset,
        });
    }
    tokens.push(Token {
        kind: Kind::Eof,
        span: source.len()..source.len(),
    });
    Ok(tokens)
}

fn line_comment_end(source: &str, mut offset: usize) -> usize {
    while offset < source.len() && !source[offset..].starts_with(['\n', '\r']) {
        offset += source[offset..].chars().next().unwrap().len_utf8();
    }
    offset
}

fn quoted_end(source: &str, start: usize, quote: char) -> Result<usize, Error> {
    let mut offset = start + quote.len_utf8();
    while let Some(ch) = source[offset..].chars().next() {
        offset += ch.len_utf8();
        if ch == quote {
            return Ok(offset);
        }
        if ch == '\\'
            && quote != '`'
            && let Some(ch) = source[offset..].chars().next()
        {
            offset += ch.len_utf8();
        }
    }
    Err(Error::new(
        source,
        start..offset,
        "unterminated quoted fragment",
    ))
}

/// Scan one balanced opaque computation block. Strings and comments shield
/// delimiters. This is deliberately not a parser or evaluator for computation.
fn code_end(source: &str, start: usize) -> Result<usize, Error> {
    let mut offset = start + 1;
    let mut closers = vec![if source[start..].starts_with('(') {
        ')'
    } else {
        '}'
    }];
    while let Some(ch) = source[offset..].chars().next() {
        let position = offset;
        offset += ch.len_utf8();
        match ch {
            '"' | '\'' | '`' => offset = quoted_end(source, position, ch)?,
            '/' if source[offset..].starts_with('/') => offset = line_comment_end(source, offset),
            '/' if source[offset..].starts_with('*') => {
                offset += 1;
                let mut depth = 1;
                while offset < source.len() && depth > 0 {
                    if source[offset..].starts_with("/*") {
                        depth += 1;
                        offset += 2;
                    } else if source[offset..].starts_with("*/") {
                        depth -= 1;
                        offset += 2;
                    } else {
                        offset += source[offset..].chars().next().unwrap().len_utf8();
                    }
                }
                if depth != 0 {
                    return Err(Error::new(
                        source,
                        position..offset,
                        "unterminated computation comment",
                    ));
                }
            }
            '(' => closers.push(')'),
            '[' => closers.push(']'),
            '{' => closers.push('}'),
            ')' | ']' | '}' => {
                if closers.pop() != Some(ch) {
                    return Err(Error::new(
                        source,
                        position..offset,
                        "unbalanced computation fragment",
                    ));
                }
                if closers.is_empty() {
                    return Ok(offset);
                }
            }
            _ => {}
        }
        if closers.len() > MAX_DEPTH {
            return Err(Error::new(
                source,
                position..offset,
                "computation nesting exceeds 64 levels",
            ));
        }
    }
    Err(Error::new(
        source,
        start..offset,
        "unterminated computation fragment",
    ))
}

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Token>,
    pos: usize,
    depth: usize,
    nodes: usize,
    building: bool,
    layer: Layer,
}

impl Parser<'_> {
    fn token(&self) -> &Token {
        &self.tokens[self.pos]
    }
    fn previous_end(&self) -> usize {
        self.tokens[self.pos.saturating_sub(1)].span.end
    }
    fn error(&self, message: impl Into<String>) -> Error {
        Error::new(self.source, self.token().span.clone(), message)
    }
    fn is_symbol(&self, symbol: char) -> bool {
        self.token().kind == Kind::Symbol(symbol)
    }
    fn take(&mut self, symbol: char) -> bool {
        if self.is_symbol(symbol) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, symbol: char) -> Result<(), Error> {
        if self.take(symbol) {
            Ok(())
        } else {
            Err(self.error(format!("expected `{symbol}`")))
        }
    }
    fn take_word(&mut self, word: &str) -> bool {
        if matches!(&self.token().kind, Kind::Word(w) if w == word) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
    fn word(&mut self) -> Result<String, Error> {
        if let Kind::Word(word) = &self.token().kind {
            let word = word.clone();
            self.pos += 1;
            Ok(word)
        } else {
            Err(self.error("expected a name or count"))
        }
    }
    fn name(&mut self) -> Result<String, Error> {
        if matches!(&self.token().kind, Kind::Word(word) if word.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_'))
        {
            self.word()
        } else {
            Err(self.error("expected a name"))
        }
    }
    fn literal(&mut self) -> Result<String, Error> {
        if let Kind::Literal(text) = &self.token().kind {
            let text = text.clone();
            self.pos += 1;
            Ok(text)
        } else {
            Err(self.error("expected a backtick-delimited terminal"))
        }
    }
    fn newlines(&mut self) {
        while matches!(self.token().kind, Kind::Newline) {
            self.pos += 1;
        }
    }
    fn declaration(&self) -> bool {
        if !self.source[..self.token().span.start]
            .rsplit(['\n', '\r'])
            .next()
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            return false;
        }
        if self.is_symbol('@')
            && matches!(&self.tokens.get(self.pos + 1).map(|t| &t.kind), Some(Kind::Word(word)) if word == "root")
        {
            return true;
        }
        let mut position = self.pos;
        if matches!(&self.tokens[position].kind, Kind::Word(word) if ["lex", "syntax", "ast", "import"].contains(&word.as_str()))
        {
            return true;
        }
        if !matches!(self.tokens[position].kind, Kind::Word(_)) {
            return false;
        }
        position += 1;
        if matches!(self.tokens[position].kind, Kind::Arguments(_)) {
            position += 1;
        }
        self.tokens[position].kind == Kind::Symbol('-')
            && self
                .tokens
                .get(position + 1)
                .is_some_and(|t| t.kind == Kind::Symbol('>'))
    }
    fn terminator(&mut self) -> Result<(), Error> {
        if self.take(';') || matches!(self.token().kind, Kind::Eof) || self.declaration() {
            Ok(())
        } else if matches!(self.token().kind, Kind::Newline) {
            self.newlines();
            if matches!(self.token().kind, Kind::Eof) || self.declaration() {
                Ok(())
            } else {
                Err(self.error("expected `;` or the next production"))
            }
        } else {
            Err(self.error("expected `;` or the next production"))
        }
    }
    fn node(&mut self, start: usize, kind: ExprKind) -> Result<Expr, Error> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(self.error("grammar exceeds the 4096 expression limit"));
        }
        Ok(Expr {
            kind,
            span: start..self.previous_end(),
        })
    }
    fn expression(&mut self) -> Result<Expr, Error> {
        let first = self.sequence()?;
        let start = first.span.start;
        let mut alternatives = vec![first];
        while self.take('|') {
            alternatives.push(self.sequence()?);
        }
        if alternatives.len() == 1 {
            Ok(alternatives.pop().unwrap())
        } else {
            self.node(start, ExprKind::OrderedChoice(alternatives))
        }
    }
    fn sequence(&mut self) -> Result<Expr, Error> {
        self.newlines();
        let start = self.token().span.start;
        let mut expressions = Vec::new();
        loop {
            self.newlines();
            if matches!(self.token().kind, Kind::Eof | Kind::Yield)
                || ['|', ')', ']', ',', ';'].iter().any(|c| self.is_symbol(*c))
                || self.declaration()
            {
                break;
            }
            expressions.push(self.prefix()?);
        }
        if expressions.is_empty() {
            return Err(self.error("expected an expression; use ε for an empty match"));
        }
        if matches!(expressions.last().unwrap().kind, ExprKind::Commit) {
            return Err(self.error("expected an expression after the commit point"));
        }
        if expressions.len() == 1 {
            Ok(expressions.pop().unwrap())
        } else {
            let end = expressions.last().unwrap().span.end;
            let mut result = self.node(start, ExprKind::Sequence(expressions))?;
            result.span.end = end;
            Ok(result)
        }
    }
    fn prefix(&mut self) -> Result<Expr, Error> {
        if self.depth >= MAX_DEPTH {
            return Err(self.error("grammar nesting exceeds 64 levels"));
        }
        self.depth += 1;
        let result = self.prefix_inner();
        self.depth -= 1;
        result
    }
    fn prefix_inner(&mut self) -> Result<Expr, Error> {
        let start = self.token().span.start;
        if self.building
            && (self.is_symbol('&')
                || self.is_symbol('!')
                || self.is_symbol('@')
                || self.is_symbol('^')
                || self.is_symbol('~'))
        {
            return Err(self.error("matching operations are not allowed in a construction result"));
        }
        if self.is_symbol('&') || self.is_symbol('!') {
            let positive = self.take('&');
            if !positive {
                self.expect('!')?;
            }
            let expression = Box::new(self.prefix()?);
            return self.node(
                start,
                ExprKind::Lookahead {
                    positive,
                    expression,
                },
            );
        }
        if matches!(self.token().kind, Kind::Word(_))
            && self
                .tokens
                .get(self.pos + 1)
                .is_some_and(|t| t.kind == Kind::Symbol(':'))
        {
            if self.building {
                return Err(self.error("capture bindings belong to the matching expression"));
            }
            let name = self.name()?;
            self.expect(':')?;
            let expression = Box::new(self.prefix()?);
            return self.node(start, ExprKind::Capture { name, expression });
        }
        let mut expression = self.atom()?;
        if matches!(expression.kind, ExprKind::Commit) {
            if self.repetition_suffix() {
                return Err(self.error("a commit point must be a standalone sequence item"));
            }
            return Ok(expression);
        }
        let bounds = if self.take('?') {
            Some(("0".into(), Some("1".into()), RangeLimit::Closed, None))
        } else if self.take('*') {
            Some(("0".into(), None, RangeLimit::HalfOpen, None))
        } else if self.take('+') {
            Some(("1".into(), None, RangeLimit::HalfOpen, None))
        } else if self.braced_repeat() {
            Some(self.braced_bounds()?)
        } else {
            None
        };
        if let Some((min, max, limit, count)) = bounds {
            let at_most_one = matches!(
                (max.as_deref(), limit),
                (Some("0" | "1"), RangeLimit::Closed) | (Some("1" | "2"), RangeLimit::HalfOpen)
            );
            if !self.building && !at_most_one && definitely_zero_width(&expression) {
                return Err(self.error("cannot repeat a zero-width expression"));
            }
            expression = self.node(
                start,
                ExprKind::Repeat {
                    expression: Box::new(expression),
                    min,
                    max,
                    limit,
                    count,
                },
            )?;
        }
        if self.repetition_suffix() {
            return Err(self.error("multiple repetition suffixes require explicit grouping"));
        }
        let suffix = self.suffix()?;
        let footnote = if self.is_symbol('[')
            && self
                .tokens
                .get(self.pos + 1)
                .is_some_and(|t| t.kind == Kind::Symbol('^'))
        {
            self.pos += 2;
            let start = self.token().span.start;
            while !self.is_symbol(']') {
                if matches!(self.token().kind, Kind::Eof | Kind::Newline) {
                    return Err(self.error("unterminated grammar footnote"));
                }
                self.pos += 1;
            }
            let text = self.source[start..self.token().span.start]
                .trim()
                .to_owned();
            if text.is_empty() {
                return Err(self.error("expected a footnote name"));
            }
            self.expect(']')?;
            Some(text)
        } else {
            None
        };
        if suffix.is_some() || footnote.is_some() {
            expression = self.node(
                start,
                ExprKind::Annotated {
                    expression: Box::new(expression),
                    suffix,
                    footnote,
                },
            )?;
        }
        Ok(expression)
    }
    fn braced_repeat(&self) -> bool {
        matches!(self.token().kind, Kind::Code(_))
            && (!self.building || self.token().span.start == self.previous_end())
    }
    fn repetition_suffix(&self) -> bool {
        ['?', '*', '+'].iter().any(|c| self.is_symbol(*c)) || self.braced_repeat()
    }
    fn braced_bounds(
        &mut self,
    ) -> Result<(String, Option<String>, RangeLimit, Option<String>), Error> {
        let Kind::Code(text) = &self.token().kind else {
            unreachable!()
        };
        let mut tokens = lex(text).map_err(|error| self.error(error.message))?;
        let end = tokens.pop().unwrap();
        tokens.push(Token {
            kind: Kind::Symbol('}'),
            span: end.span.clone(),
        });
        tokens.push(end);
        let mut bounds = Parser {
            source: text,
            tokens,
            pos: 0,
            depth: 0,
            nodes: 0,
            building: false,
            layer: Layer::Syntax,
        };
        let value = bounds
            .repeat_bounds()
            .map_err(|error| self.error(error.message))?;
        if !matches!(bounds.token().kind, Kind::Eof) {
            return Err(self.error("unexpected repetition bound"));
        }
        self.pos += 1;
        Ok(value)
    }
    fn repeat_bounds(
        &mut self,
    ) -> Result<(String, Option<String>, RangeLimit, Option<String>), Error> {
        let count = if matches!(self.token().kind, Kind::Word(_))
            && self
                .tokens
                .get(self.pos + 1)
                .is_some_and(|t| t.kind == Kind::Symbol(':'))
        {
            let name = self.name()?;
            self.expect(':')?;
            Some(name)
        } else {
            None
        };
        let first = if matches!(self.token().kind, Kind::Word(_)) {
            Some(self.word()?)
        } else {
            None
        };
        if self.take('.') {
            self.expect('.')?;
            let limit = if self.take('=') {
                RangeLimit::Closed
            } else {
                RangeLimit::HalfOpen
            };
            let min = first.unwrap_or_else(|| "0".into());
            let minimum = min
                .parse::<u32>()
                .map_err(|_| self.error("range bounds must be u32 integer counts"))?;
            let max = if matches!(self.token().kind, Kind::Word(_)) {
                Some(self.word()?)
            } else {
                None
            };
            if limit == RangeLimit::Closed && max.is_none() {
                return Err(self.error("inclusive repetition requires an upper bound"));
            }
            if let Some(max) = &max {
                let maximum = max
                    .parse::<u32>()
                    .map_err(|_| self.error("range bounds must be u32 integer counts"))?;
                if maximum < minimum || (limit == RangeLimit::HalfOpen && maximum == minimum) {
                    return Err(self.error("empty or reversed repetition interval"));
                }
            }
            self.expect('}')?;
            Ok((min, max, limit, count))
        } else {
            if count.is_some() {
                return Err(self.error("a bound repeat count requires an interval"));
            }
            let count =
                first.ok_or_else(|| self.error("expected a repetition count or interval"))?;
            if count.chars().all(|c| c.is_ascii_digit()) {
                count
                    .parse::<u32>()
                    .map_err(|_| self.error("repetition count exceeds u32"))?;
            } else if !count
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
            {
                return Err(self.error("expected a repetition count name"));
            }
            self.expect('}')?;
            Ok((count.clone(), Some(count), RangeLimit::Closed, None))
        }
    }
    fn atom(&mut self) -> Result<Expr, Error> {
        let start = self.token().span.start;
        let kind = match self.token().kind.clone() {
            Kind::Word(word)
                if self.building && ["let", "scan", "probe", "within"].contains(&word.as_str()) =>
            {
                return Err(
                    self.error("matching operations are not allowed in a construction result")
                );
            }
            Kind::Code(_) if self.building => ExprKind::Computation(self.code()?),
            Kind::Literal(text) => {
                self.pos += 1;
                ExprKind::Literal(text)
            }
            Kind::Unicode(character, hex) => {
                self.pos += 1;
                ExprKind::Unicode { character, hex }
            }
            Kind::Prose(text) => {
                self.pos += 1;
                ExprKind::Prose(text)
            }
            Kind::Word(word) if word == "ε" => {
                self.pos += 1;
                ExprKind::Empty
            }
            Kind::Word(word) if word == "let" && !self.building => {
                self.pos += 1;
                let name = self.name()?;
                self.expect('=')?;
                let value = self.code()?;
                ExprKind::Binding { name, value }
            }
            Kind::Word(word) if (word == "scan" || word == "probe") && !self.building => {
                self.pos += 1;
                let name = self.name()?;
                let arguments = self.arguments()?;
                ExprKind::Scanner {
                    probe: word == "probe",
                    name,
                    arguments,
                }
            }
            Kind::Word(word) if word == "within" && !self.building => {
                self.pos += 1;
                self.expect('(')?;
                let end_start = self.token().span.start;
                self.raw_until(',')?;
                let end = self.source[end_start..self.token().span.start]
                    .trim()
                    .to_owned();
                if end.is_empty() {
                    return Err(self.error("expected a container boundary"));
                }
                self.expect(',')?;
                let expression = Box::new(self.expression()?);
                self.expect(')')?;
                ExprKind::Within { end, expression }
            }
            Kind::Word(_) => {
                let mut name = self.name()?;
                while self.take('.') {
                    name.push('.');
                    name.push_str(&self.name()?);
                }
                let arguments = self.arguments()?;
                if (self.building || self.layer == Layer::Ast)
                    && self.is_symbol('[')
                    && !self
                        .tokens
                        .get(self.pos + 1)
                        .is_some_and(|token| token.kind == Kind::Symbol('^'))
                {
                    self.pos += 1;
                    self.newlines();
                    let children = if self.is_symbol(']') {
                        self.node(self.token().span.start, ExprKind::Empty)?
                    } else {
                        self.expression()?
                    };
                    self.expect(']')?;
                    ExprKind::Node {
                        name,
                        arguments,
                        children: Box::new(children),
                    }
                } else {
                    ExprKind::Reference { name, arguments }
                }
            }
            Kind::Symbol('(') => {
                self.pos += 1;
                let mut expression = self.expression()?;
                self.expect(')')?;
                expression.span = start..self.previous_end();
                return Ok(expression);
            }
            Kind::Symbol('[') => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.newlines();
                    if self.take(']') {
                        break;
                    }
                    self.nodes += 1;
                    if self.nodes > MAX_NODES {
                        return Err(self.error("grammar exceeds the 4096 expression limit"));
                    }
                    if let Kind::Word(_) = self.token().kind {
                        items.push(CharacterSetItem::Reference(self.name()?));
                        continue;
                    }
                    let first = self.character()?;
                    if self.take('-') {
                        let last = self.character()?;
                        if first > last {
                            return Err(self.error("character range is reversed"));
                        }
                        items.push(CharacterSetItem::Range(first, last));
                    } else {
                        items.push(CharacterSetItem::Character(first));
                    }
                }
                if items.is_empty() {
                    return Err(self.error("character set must not be empty"));
                }
                ExprKind::CharacterSet(items)
            }
            Kind::Symbol('~') => {
                self.pos += 1;
                // Negation binds before the repetition suffix, unlike lookahead.
                if !matches!(
                    self.token().kind,
                    Kind::Literal(_) | Kind::Unicode(..) | Kind::Word(_) | Kind::Symbol('[')
                ) {
                    return Err(
                        self.error("~ requires a terminal, Unicode scalar, rule or character set")
                    );
                }
                let excluded = self.atom()?;
                if !matches!(
                    excluded.kind,
                    ExprKind::Literal(_)
                        | ExprKind::Unicode { .. }
                        | ExprKind::Reference { .. }
                        | ExprKind::CharacterSet(_)
                ) {
                    return Err(
                        self.error("~ requires a terminal, Unicode scalar, rule or character set")
                    );
                }
                ExprKind::Complement(Box::new(excluded))
            }
            Kind::Symbol('@') => {
                self.pos += 1;
                ExprKind::Predicate(self.code()?)
            }
            Kind::Symbol('^') => {
                self.pos += 1;
                ExprKind::Commit
            }
            _ => return Err(self.error("unexpected grammar expression")),
        };
        self.node(start, kind)
    }
    fn code(&mut self) -> Result<String, Error> {
        if let Kind::Code(code) = &self.token().kind {
            if code.is_empty() {
                return Err(self.error("expected a nonempty computation fragment"));
            }
            let code = code.clone();
            self.pos += 1;
            Ok(code)
        } else {
            Err(self.error("expected a computation fragment in `{ ... }`"))
        }
    }
    fn character(&mut self) -> Result<char, Error> {
        if let Kind::Unicode(character, _) = self.token().kind {
            self.pos += 1;
            return Ok(character);
        }
        if let Kind::Literal(text) = &self.token().kind {
            let mut chars = text.chars();
            if let Some(ch) = chars.next()
                && chars.next().is_none()
            {
                self.pos += 1;
                return Ok(ch);
            }
        }
        Err(self.error("character set entries must contain one Unicode scalar"))
    }
    fn arguments(&mut self) -> Result<Option<String>, Error> {
        // Rust-style grouping remains distinct from our adjacent rule invocation.
        if let Kind::Arguments(arguments) = &self.token().kind {
            let arguments = arguments.clone();
            self.pos += 1;
            Ok(Some(arguments))
        } else {
            Ok(None)
        }
    }
    fn raw_until(&mut self, delimiter: char) -> Result<(), Error> {
        let mut closers = Vec::new();
        loop {
            match self.token().kind {
                Kind::Eof => return Err(self.error(format!("expected `{delimiter}`"))),
                Kind::Symbol(c) if c == delimiter && closers.is_empty() => return Ok(()),
                Kind::Symbol('(') => closers.push(')'),
                Kind::Symbol('[') => closers.push(']'),
                Kind::Symbol('{') => closers.push('}'),
                Kind::Symbol(c @ (')' | ']' | '}')) if closers.pop() != Some(c) => {
                    return Err(self.error("unbalanced invocation arguments"));
                }
                _ => {}
            }
            if closers.len() > MAX_DEPTH {
                return Err(self.error("argument nesting exceeds 64 levels"));
            }
            self.pos += 1;
        }
    }
    fn suffix(&mut self) -> Result<Option<String>, Error> {
        let start = self.token().span.start;
        if !self.source[start..].starts_with('_')
            || start == self.previous_end()
            || self.source[self.previous_end()..start].contains(['\r', '\n'])
        {
            return Ok(None);
        }
        let mut offset = start + 1;
        while let Some(ch) = self.source[offset..].chars().next() {
            if ch == '_' {
                let text = self.source[start + 1..offset].trim().to_owned();
                if text.is_empty() {
                    return Err(self.error("empty grammar suffix"));
                }
                offset += 1;
                while self.token().span.end <= offset && !matches!(self.token().kind, Kind::Eof) {
                    self.pos += 1;
                }
                if self.token().span.start < offset {
                    return Err(self.error("closing suffix underscore must end a token"));
                }
                return Ok(Some(text));
            }
            if ch == '\r' || ch == '\n' {
                break;
            }
            if ch == '`' {
                offset = quoted_end(self.source, offset, '`')?;
            } else {
                offset += ch.len_utf8();
            }
        }
        Err(self.error("unterminated grammar suffix"))
    }
}

fn definitely_zero_width(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::Lookahead { .. }
        | ExprKind::Predicate(_)
        | ExprKind::Binding { .. }
        | ExprKind::Commit
        | ExprKind::Empty
        | ExprKind::Scanner { probe: true, .. } => true,
        ExprKind::Capture { expression, .. }
        | ExprKind::Within { expression, .. }
        | ExprKind::Annotated { expression, .. } => definitely_zero_width(expression),
        ExprKind::Sequence(es) | ExprKind::OrderedChoice(es) => {
            es.iter().all(definitely_zero_width)
        }
        ExprKind::Repeat {
            expression,
            max,
            limit,
            ..
        } => {
            (max.as_deref() == Some("0") && *limit == RangeLimit::Closed)
                || definitely_zero_width(expression)
        }
        _ => false,
    }
}
