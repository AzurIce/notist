use rowan::{TextRange, TextSize};

use notist_syntax::parser::Diagnostic;

use crate::item::{Dict, Value};

/// Parse the payload of an annotation `@(…)`: dict entries without the
/// enclosing parens.
pub fn parse_dict_entries(src: &str, base: u32) -> (Dict, Vec<Diagnostic>) {
    let mut p = LitParser::new(src, base);
    let mut dict = Dict::default();
    for entry in p.entries() {
        match entry {
            Entry::Named(k, v) => dict.insert(k, v),
            Entry::Pos(_) => p.error_here("annotation entries must be `key: value`"),
        }
    }
    (dict, p.diags)
}

/// Parse a call's argument list: positional literals and `key: value` named
/// arguments.
pub fn parse_args(src: &str, base: u32) -> (Vec<Value>, Dict, Vec<Diagnostic>) {
    let mut p = LitParser::new(src, base);
    let mut positional = Vec::new();
    let mut named = Dict::default();
    for entry in p.entries() {
        match entry {
            Entry::Pos(v) => positional.push(v),
            Entry::Named(k, v) => named.insert(k, v),
        }
    }
    (positional, named, p.diags)
}

enum Entry {
    Pos(Value),
    Named(String, Value),
}

/// A recursive-descent parser over a source slice for the literal subgrammar
/// (the M1 code mode). Diagnostics carry absolute spans via `base`.
struct LitParser<'a> {
    src: &'a str,
    pos: usize,
    base: u32,
    diags: Vec<Diagnostic>,
}

impl<'a> LitParser<'a> {
    fn new(src: &'a str, base: u32) -> Self {
        Self {
            src,
            pos: 0,
            base,
            diags: Vec::new(),
        }
    }

    fn entries(&mut self) -> Vec<Entry> {
        let mut entries = Vec::new();
        loop {
            self.skip_ws();
            if self.eof() {
                return entries;
            }
            match self.entry() {
                Some(entry) => entries.push(entry),
                None => self.recover(),
            }
            self.skip_ws();
            if !self.eat(b',') {
                break;
            }
        }
        if !self.eof() {
            self.error_here("expected `,` between entries");
        }
        entries
    }

    fn entry(&mut self) -> Option<Entry> {
        self.skip_ws();
        // key candidate: quoted string or bare identifier, then `:`
        let mark = self.pos;
        let quoted = self.at(b'"') || self.at_raw_string();
        let candidate = if quoted {
            self.string()
        } else if self.at_ident_start() {
            Some(self.ident())
        } else {
            None
        };
        if let Some(key) = candidate {
            let after = self.pos;
            self.skip_ws();
            if self.eat(b':') {
                let value = self.value()?;
                return Some(Entry::Named(key, value));
            }
            if quoted {
                return Some(Entry::Pos(Value::Str(key)));
            }
            self.pos = after;
            return match key.as_str() {
                "true" => Some(Entry::Pos(Value::Bool(true))),
                "false" => Some(Entry::Pos(Value::Bool(false))),
                _ => {
                    self.error_at(mark, "bare names are only valid as keys");
                    None
                }
            };
        }
        Some(Entry::Pos(self.value()?))
    }

    fn value(&mut self) -> Option<Value> {
        self.skip_ws();
        match self.peek()? {
            b'(' => return self.paren(),
            b'"' => return self.string().map(Value::Str),
            b'r' if self.at_raw_string() => return self.string().map(Value::Str),
            b'-' | b'0'..=b'9' => return self.number(),
            _ => {}
        }
        if self.at_ident_start() {
            let name = self.ident();
            return match name.as_str() {
                "true" => Some(Value::Bool(true)),
                "false" => Some(Value::Bool(false)),
                _ => {
                    self.error_here("expected a literal");
                    None
                }
            };
        }
        self.error_here("expected a literal");
        None
    }

    /// `(…)` — unit / array / dict / grouping, discriminated locally: any
    /// `k: v` entry makes a dict, a comma makes an array, a lone value with
    /// no comma is grouping.
    fn paren(&mut self) -> Option<Value> {
        self.expect(b'(');
        self.skip_ws();
        if self.eat(b')') {
            return Some(Value::Unit);
        }
        if self.eat(b':') {
            self.skip_ws();
            self.expect(b')');
            return Some(Value::Dict(Dict::default()));
        }
        if self.eat(b',') {
            self.skip_ws();
            self.expect(b')');
            return Some(Value::Array(Vec::new()));
        }
        let mut positional = Vec::new();
        let mut named = Dict::default();
        let mut trailing_comma = false;
        loop {
            self.skip_ws();
            if self.eat(b')') {
                break;
            }
            match self.entry() {
                Some(Entry::Pos(v)) => {
                    if named.iter().next().is_some() {
                        self.error_here("positional entry after a named entry");
                    }
                    positional.push(v);
                }
                Some(Entry::Named(k, v)) => named.insert(k, v),
                None => self.recover(),
            }
            trailing_comma = false;
            self.skip_ws();
            if self.eat(b',') {
                trailing_comma = true;
                continue;
            }
            self.expect(b')');
            break;
        }
        let has_named = named.iter().next().is_some();
        if has_named && !positional.is_empty() {
            self.error_here("cannot mix positional and named entries in a dict");
        }
        match (positional.len(), has_named) {
            (_, true) => Some(Value::Dict(named)),
            (1, false) if !trailing_comma => Some(positional.pop().unwrap()),
            (_, false) => Some(Value::Array(positional)),
        }
    }

    /// String literals, four forms: `"…"` / `"""…"""` / `r#"…"#` /
    /// `r#"""…"""#`. Escapes are exactly `\" \\ \n \r \t`; raw forms take no
    /// escapes. Multiline forms require a newline right after the opener
    /// (not part of the value) and trim one newline before the closer.
    fn string(&mut self) -> Option<String> {
        if self.at_raw_string() {
            return self.raw_string();
        }
        self.expect(b'"');
        if self.at(b'"') && self.at_offset(1, b'"') {
            return self.multiline_string();
        }
        let mut out = String::new();
        loop {
            match self.peek() {
                None | Some(b'\n') => {
                    self.error_here("unclosed string");
                    return None;
                }
                Some(b'"') => {
                    self.pos += 1;
                    return Some(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match self.peek() {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        _ => {
                            self.error_here("unknown escape");
                            return None;
                        }
                    }
                    self.pos += 1;
                }
                Some(_) => {
                    let ch = self.src[self.pos..].chars().next().unwrap();
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    fn multiline_string(&mut self) -> Option<String> {
        self.pos += 3;
        if !self.eat(b'\n') {
            self.error_here("multiline string opener must be followed by a newline");
            return None;
        }
        let start = self.pos;
        let close = self.src[start..].find("\"\"\"").map(|i| start + i);
        let Some(close) = close else {
            self.error_here("unclosed multiline string");
            return None;
        };
        let mut text = &self.src[start..close];
        if text.ends_with('\n') {
            text = &text[..text.len() - 1];
        }
        let out = self.unescape(text, start)?;
        self.pos = close + 3;
        Some(out)
    }

    fn raw_string(&mut self) -> Option<String> {
        self.expect(b'r');
        let hashes = self.src[self.pos..].bytes().take_while(|&b| b == b'#').count();
        self.pos += hashes;
        if hashes == 0 || !self.eat(b'"') {
            self.error_here("raw string needs at least one `#`");
            return None;
        }
        let multiline = self.at(b'"') && self.at_offset(1, b'"');
        if multiline {
            self.pos += 2;
            if !self.eat(b'\n') {
                self.error_here("multiline string opener must be followed by a newline");
                return None;
            }
        }
        let closer = format!("\"{}", "#".repeat(hashes));
        let start = self.pos;
        let close = self.src[start..].find(&closer).map(|i| start + i);
        let Some(mut close) = close else {
            self.error_here("unclosed raw string");
            return None;
        };
        if multiline && self.src[start..close].ends_with('\n') {
            close -= 1;
        }
        let out = self.src[start..close].to_string();
        self.pos = close + closer.len();
        Some(out)
    }

    fn unescape(&mut self, text: &str, start: usize) -> Option<String> {
        let mut out = String::new();
        let mut chars = text.char_indices().peekable();
        while let Some((i, ch)) = chars.next() {
            if ch != '\\' {
                out.push(ch);
                continue;
            }
            match chars.next() {
                Some((_, '"')) => out.push('"'),
                Some((_, '\\')) => out.push('\\'),
                Some((_, 'n')) => out.push('\n'),
                Some((_, 'r')) => out.push('\r'),
                Some((_, 't')) => out.push('\t'),
                _ => {
                    self.error_at(start + i, "unknown escape");
                    return None;
                }
            }
        }
        Some(out)
    }

    /// Int / Float; a leading `-` folds into the literal here (it is a unary
    /// operator in the language, but the literal subgrammar has no
    /// operators).
    fn number(&mut self) -> Option<Value> {
        let negative = self.eat(b'-');
        let int_start = self.pos;
        self.pos += self.src[self.pos..].bytes().take_while(|b| b.is_ascii_digit()).count();
        if self.pos == int_start {
            self.error_here("expected digits");
            return None;
        }
        let mut float = false;
        if self.at(b'.') && self.src[self.pos + 1..].bytes().next().is_some_and(|b| b.is_ascii_digit())
        {
            float = true;
            self.pos += 1;
            self.pos +=
                self.src[self.pos..].bytes().take_while(|b| b.is_ascii_digit()).count();
        }
        let text: String = self.src[int_start..self.pos].to_string();
        let text = if negative { format!("-{text}") } else { text };
        if float {
            text.parse::<f64>().ok().map(Value::Float)
        } else {
            text.parse::<i64>().ok().map(Value::Int)
        }
        .or_else(|| {
            self.error_here("number out of range");
            None
        })
    }

    fn ident(&mut self) -> String {
        let start = self.pos;
        while let Some(ch) = self.src[self.pos..].chars().next() {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                self.pos += ch.len_utf8();
            } else {
                break;
            }
        }
        self.src[start..self.pos].to_string()
    }

    fn at_ident_start(&self) -> bool {
        self.src[self.pos..]
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
    }

    fn at_raw_string(&self) -> bool {
        if !self.at(b'r') {
            return false;
        }
        let rest = &self.src[self.pos + 1..];
        let hashes = rest.bytes().take_while(|&b| b == b'#').count();
        hashes >= 1 && rest[hashes..].starts_with('"')
    }

    fn skip_ws(&mut self) {
        self.pos += self.src[self.pos..]
            .bytes()
            .take_while(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
            .count();
    }

    fn recover(&mut self) {
        while let Some(ch) = self.src[self.pos..].chars().next() {
            if ch == ',' || ch == ')' {
                return;
            }
            self.pos += ch.len_utf8();
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.as_bytes().get(self.pos).copied()
    }

    fn at(&self, b: u8) -> bool {
        self.peek() == Some(b)
    }

    fn at_offset(&self, offset: usize, b: u8) -> bool {
        self.src.as_bytes().get(self.pos + offset) == Some(&b)
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.at(b) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, b: u8) {
        if !self.eat(b) {
            self.error_here(&format!("expected `{}`", b as char));
        }
    }

    fn eof(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn error_here(&mut self, message: &str) {
        self.error_at(self.pos, message);
    }

    fn error_at(&mut self, pos: usize, message: &str) {
        let point = TextSize::new(self.base + pos as u32);
        self.diags.push(Diagnostic {
            span: TextRange::new(point, point),
            message: message.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(src: &str) -> (Vec<Value>, Dict, Vec<Diagnostic>) {
        parse_args(src, 0)
    }

    #[test]
    fn scalars() {
        let (pos, named, diags) = args("1, 2.5, true, \"x\"");
        assert!(diags.is_empty());
        assert_eq!(
            pos,
            vec![
                Value::Int(1),
                Value::Float(2.5),
                Value::Bool(true),
                Value::Str("x".to_string()),
            ]
        );
        assert!(named.iter().next().is_none());
    }

    #[test]
    fn named() {
        let (_, named, diags) = args("kind: \"warning\", level: 2");
        assert!(diags.is_empty());
        assert_eq!(named.get("kind"), Some(&Value::Str("warning".to_string())));
        assert_eq!(named.get("level"), Some(&Value::Int(2)));
    }

    #[test]
    fn collections() {
        let (pos, _, diags) = args("(1, 2), (3,), (\"k\": 1), (:), (), (4)");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(
            pos,
            vec![
                Value::Array(vec![Value::Int(1), Value::Int(2)]),
                Value::Array(vec![Value::Int(3)]),
                Value::Dict(Dict::default().tap("k", Value::Int(1))),
                Value::Dict(Dict::default()),
                Value::Unit,
                Value::Int(4),
            ]
        );
    }

    #[test]
    fn strings() {
        let (pos, _, diags) = args("\"a\\nb\", r#\"raw\\n\"#, -3.5");
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(pos[0], Value::Str("a\nb".to_string()));
        assert_eq!(pos[1], Value::Str("raw\\n".to_string()));
        assert_eq!(pos[2], Value::Float(-3.5));
    }

    #[test]
    fn bare_name_is_not_a_value() {
        let (_, _, diags) = args("warning");
        assert_eq!(diags.len(), 1);
    }

    trait Tap {
        fn tap(self, key: &str, value: Value) -> Self;
    }
    impl Tap for Dict {
        fn tap(mut self, key: &str, value: Value) -> Self {
            self.insert(key, value);
            self
        }
    }
}

