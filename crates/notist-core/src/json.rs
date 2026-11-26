//! Zero-dependency, immutable JSON value model and read-only `DataHost` trait for `notist-core`.

use std::fmt;
use std::sync::Arc;

/// Read-only external data host supplied by an embedder (such as `notist-ssg`)
/// during document evaluation. `notist-core` itself performs zero filesystem IO.
pub trait DataHost: Sync {
    /// Read and parse a JSON document located at `rel_path` relative to `from_doc`.
    fn read_json(&self, from_doc: &str, rel_path: &str) -> Result<JsonValue, String>;

    /// Read a UTF-8 plain-text file located at `rel_path` relative to `from_doc`.
    fn read_text(&self, _from_doc: &str, rel_path: &str) -> Result<String, String> {
        Err(format!(
            "cannot read text `{rel_path}`: data host does not support text reads"
        ))
    }
}

/// Default `DataHost` that rejects external data reads.
pub struct NoDataHost;

impl DataHost for NoDataHost {
    fn read_json(&self, _from_doc: &str, rel_path: &str) -> Result<JsonValue, String> {
        Err(format!(
            "cannot read JSON `{rel_path}`: no data host configured"
        ))
    }

    fn read_text(&self, _from_doc: &str, rel_path: &str) -> Result<String, String> {
        Err(format!(
            "cannot read text `{rel_path}`: no data host configured"
        ))
    }
}

/// Shared, immutable JSON value backed by reference-counted storage.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonValue(Arc<JsonData>);

#[derive(Debug, Clone, PartialEq)]
pub enum JsonData {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Arc<str>),
    Array(Arc<[JsonValue]>),
    Object(Arc<[(Arc<str>, JsonValue)]>),
}

impl JsonValue {
    pub fn null() -> Self {
        Self(Arc::new(JsonData::Null))
    }

    pub fn from_data(data: JsonData) -> Self {
        Self(Arc::new(data))
    }

    pub fn data(&self) -> &JsonData {
        &self.0
    }

    pub fn parse(input: &str) -> Result<Self, String> {
        let mut parser = JsonParser::new(input);
        let value = parser.parse_value(0)?;
        parser.skip_ws();
        if parser.pos < parser.bytes.len() {
            return Err(format!("trailing JSON characters at byte {}", parser.pos));
        }
        Ok(value)
    }

    pub fn is_null(&self) -> bool {
        matches!(self.0.as_ref(), JsonData::Null)
    }

    pub fn get(&self, key: &str) -> JsonValue {
        if let JsonData::Object(entries) = self.0.as_ref() {
            for (k, v) in entries.iter() {
                if k.as_ref() == key {
                    return v.clone();
                }
            }
        }
        Self::null()
    }

    pub fn at(&self, index: i64) -> JsonValue {
        if index >= 0
            && let JsonData::Array(items) = self.0.as_ref()
            && let Some(item) = items.get(index as usize)
        {
            return item.clone();
        }
        Self::null()
    }

    pub fn as_string(&self, default: &str) -> Arc<str> {
        match self.0.as_ref() {
            JsonData::String(s) => Arc::clone(s),
            _ => Arc::from(default),
        }
    }

    pub fn as_int(&self, default: i64) -> i64 {
        match self.0.as_ref() {
            JsonData::Int(n) => *n,
            _ => default,
        }
    }

    pub fn as_bool(&self, default: bool) -> bool {
        match self.0.as_ref() {
            JsonData::Bool(b) => *b,
            _ => default,
        }
    }

    pub fn as_list(&self) -> Arc<[JsonValue]> {
        match self.0.as_ref() {
            JsonData::Array(items) => Arc::clone(items),
            _ => Arc::from([]),
        }
    }

    pub fn len(&self) -> i64 {
        match self.0.as_ref() {
            JsonData::Array(items) => items.len() as i64,
            JsonData::Object(entries) => entries.len() as i64,
            JsonData::String(s) => s.len() as i64,
            _ => 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Display for JsonValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.as_ref() {
            JsonData::Null => write!(f, "null"),
            JsonData::Bool(b) => write!(f, "{b}"),
            JsonData::Int(n) => write!(f, "{n}"),
            JsonData::Float(v) => write!(f, "{v}"),
            JsonData::String(s) => write!(f, "{s:?}"),
            JsonData::Array(items) => {
                write!(f, "[")?;
                for (idx, item) in items.iter().enumerate() {
                    if idx > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, "]")
            }
            JsonData::Object(entries) => {
                write!(f, "{{")?;
                for (idx, (k, v)) in entries.iter().enumerate() {
                    if idx > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k:?}: {v}")?;
                }
                write!(f, "}}")
            }
        }
    }
}

const MAX_JSON_DEPTH: usize = 256;

struct JsonParser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src,
            bytes: src.as_bytes(),
            pos: 0,
        }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\r' | b'\n')
        {
            self.pos += 1;
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<JsonValue, String> {
        if depth > MAX_JSON_DEPTH {
            return Err("JSON nesting depth limit exceeded".into());
        }
        self.skip_ws();
        let Some(&b) = self.bytes.get(self.pos) else {
            return Err("unexpected end of JSON input".into());
        };
        match b {
            b'n' => self.parse_literal(b"null", JsonData::Null),
            b't' => self.parse_literal(b"true", JsonData::Bool(true)),
            b'f' => self.parse_literal(b"false", JsonData::Bool(false)),
            b'"' => self
                .parse_string()
                .map(|s| JsonValue::from_data(JsonData::String(s))),
            b'[' => self.parse_array(depth + 1),
            b'{' => self.parse_object(depth + 1),
            b'-' | b'0'..=b'9' => self.parse_number(),
            other => Err(format!(
                "unexpected JSON byte {:?} at offset {}",
                other as char, self.pos
            )),
        }
    }

    fn parse_literal(&mut self, expected: &[u8], data: JsonData) -> Result<JsonValue, String> {
        if self.bytes[self.pos..].starts_with(expected) {
            self.pos += expected.len();
            Ok(JsonValue::from_data(data))
        } else {
            Err(format!("invalid JSON literal at offset {}", self.pos))
        }
    }

    fn parse_string(&mut self) -> Result<Arc<str>, String> {
        if self.bytes.get(self.pos) != Some(&b'"') {
            return Err(format!("expected `\"` at offset {}", self.pos));
        }
        self.pos += 1;
        let start = self.pos;
        // Fast path: scan for `"` or `\` with zero intermediate allocations.
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'"' => {
                    let s = &self.src[start..self.pos];
                    self.pos += 1;
                    return Ok(Arc::from(s));
                }
                b'\\' => break,
                0x00..=0x1f => {
                    return Err(format!(
                        "unescaped control character in JSON string at offset {}",
                        self.pos
                    ));
                }
                _ => self.pos += 1,
            }
        }
        // Slow path: string contains escape sequences.
        let mut out = String::from(&self.src[start..self.pos]);
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b'"' => {
                    self.pos += 1;
                    return Ok(Arc::from(out));
                }
                b'\\' => {
                    self.pos += 1;
                    let Some(&esc) = self.bytes.get(self.pos) else {
                        return Err("unterminated JSON escape".into());
                    };
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let code = self.parse_hex4()?;
                            if (0xD800..=0xDBFF).contains(&code) {
                                // High surrogate: require `\uXXXX` low surrogate.
                                if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                    return Err("unpaired high surrogate in JSON string".into());
                                }
                                self.pos += 2;
                                let low = self.parse_hex4()?;
                                if !(0xDC00..=0xDFFF).contains(&low) {
                                    return Err("invalid low surrogate in JSON string".into());
                                }
                                let combined = 0x10000
                                    + (((code as u32 - 0xD800) << 10) | (low as u32 - 0xDC00));
                                let ch = char::from_u32(combined)
                                    .ok_or_else(|| "invalid unicode scalar".to_owned())?;
                                out.push(ch);
                            } else if (0xDC00..=0xDFFF).contains(&code) {
                                return Err("unpaired low surrogate in JSON string".into());
                            } else {
                                let ch = char::from_u32(code as u32)
                                    .ok_or_else(|| "invalid unicode scalar".to_owned())?;
                                out.push(ch);
                            }
                        }
                        other => {
                            return Err(format!("invalid JSON escape `\\{}`", other as char));
                        }
                    }
                }
                0x00..=0x1f => {
                    return Err(format!(
                        "unescaped control character in JSON string at offset {}",
                        self.pos
                    ));
                }
                _ => {
                    let ch = self.src[self.pos..].chars().next().expect("valid utf-8");
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        Err("unterminated JSON string".into())
    }

    fn parse_hex4(&mut self) -> Result<u16, String> {
        let Some(slice) = self.src.get(self.pos..self.pos + 4) else {
            return Err("truncated \\uXXXX escape in JSON string".into());
        };
        let val = u16::from_str_radix(slice, 16)
            .map_err(|_| format!("invalid \\uXXXX hex escape `{slice}`"))?;
        self.pos += 4;
        Ok(val)
    }

    fn parse_number(&mut self) -> Result<JsonValue, String> {
        let start = self.pos;
        if self.bytes.get(self.pos) == Some(&b'-') {
            self.pos += 1;
        }
        match self.bytes.get(self.pos) {
            Some(b'0') => {
                self.pos += 1;
            }
            Some(b'1'..=b'9') => {
                while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
            }
            _ => return Err(format!("invalid JSON number at offset {start}")),
        }
        let mut is_float = false;
        if self.bytes.get(self.pos) == Some(&b'.') {
            is_float = true;
            self.pos += 1;
            let frac_start = self.pos;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            if self.pos == frac_start {
                return Err(format!("invalid JSON fraction at offset {start}"));
            }
        }
        if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
            is_float = true;
            self.pos += 1;
            if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            if self.pos == exp_start {
                return Err(format!("invalid JSON exponent at offset {start}"));
            }
        }
        let raw = &self.src[start..self.pos];
        if !is_float && let Ok(n) = raw.parse::<i64>() {
            return Ok(JsonValue::from_data(JsonData::Int(n)));
        }
        let f = raw
            .parse::<f64>()
            .map_err(|_| format!("invalid JSON number `{raw}`"))?;
        if !f.is_finite() {
            return Err(format!("non-finite JSON number `{raw}`"));
        }
        Ok(JsonValue::from_data(JsonData::Float(f)))
    }

    fn parse_array(&mut self, depth: usize) -> Result<JsonValue, String> {
        self.pos += 1; // skip '['
        self.skip_ws();
        let mut items = Vec::new();
        if self.bytes.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(JsonValue::from_data(JsonData::Array(Arc::from(items))));
        }
        loop {
            items.push(self.parse_value(depth)?);
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(JsonValue::from_data(JsonData::Array(Arc::from(items))));
                }
                _ => return Err(format!("expected `,` or `]` at offset {}", self.pos)),
            }
        }
    }

    fn parse_object(&mut self, depth: usize) -> Result<JsonValue, String> {
        self.pos += 1; // skip '{'
        self.skip_ws();
        let mut entries = Vec::new();
        if self.bytes.get(self.pos) == Some(&b'}') {
            self.pos += 1;
            return Ok(JsonValue::from_data(JsonData::Object(Arc::from(entries))));
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.bytes.get(self.pos) != Some(&b':') {
                return Err(format!("expected `:` at offset {}", self.pos));
            }
            self.pos += 1;
            let value = self.parse_value(depth)?;
            entries.push((key, value));
            self.skip_ws();
            match self.bytes.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(JsonValue::from_data(JsonData::Object(Arc::from(entries))));
                }
                _ => return Err(format!("expected `,` or `}}` at offset {}", self.pos)),
            }
        }
    }
}
