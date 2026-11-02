use super::*;

/// No IO and no diagnostics. Format errors are ordinary data; hard limits are
/// separate failures and cannot be swallowed as user Result values.
pub fn parse(format: DataFormat, bytes: &[u8], limits: DataLimits) -> Result<Data, DataFailure> {
    if bytes.len() > limits.source_bytes.min(u32::MAX as usize) {
        return Err(DataFailure::Limit("data source byte limit exceeded"));
    }
    let text: Arc<str> = std::str::from_utf8(bytes)
        .map_err(|_| DataError::new(DataErrorKind::Utf8, "data input is not valid UTF-8"))?
        .into();
    if text.starts_with('\u{feff}') {
        return Err(
            DataError::new(DataErrorKind::Syntax, "UTF-8 BOM is not supported")
                .located(String::new(), Some(0..3), Some(text))
                .into(),
        );
    }
    let builder = Builder {
        nodes: Vec::new(),
        text: Some(text.clone()),
        limits,
        bytes: text.len(),
    };
    if format == DataFormat::Json && json_working_set(text.as_bytes()) > limits.working_bytes {
        return Err(DataFailure::Limit("JSON parser working-set limit exceeded"));
    }
    match format {
        DataFormat::Json => {
            let mut parser = Json {
                text,
                cursor: 0,
                builder,
                path: String::new(),
            };
            let root = parser.value(0)?;
            parser.space();
            if parser.cursor != parser.text.len() {
                return Err(parser.error(
                    DataErrorKind::Syntax,
                    "unexpected trailing JSON input",
                    parser.cursor..parser.cursor + 1,
                ));
            }
            Ok(parser.builder.finish(root))
        }
        DataFormat::Toml => {
            // toml_edit retains a format DOM until conversion completes. Use a
            // deliberately conservative allowance rather than treating it as
            // zero-copy; hosts can explicitly raise this bound for large TOML.
            if text.len().saturating_mul(64) > limits.working_bytes {
                return Err(DataFailure::Limit("TOML parser working-set limit exceeded"));
            }
            let document = toml_edit::ImDocument::parse(text.clone()).map_err(|error| {
                DataError::new(DataErrorKind::Syntax, error.message().to_string()).located(
                    String::new(),
                    error.span(),
                    Some(text.clone()),
                )
            })?;
            let mut parser = Toml {
                builder: Builder {
                    limits: DataLimits {
                        working_bytes: limits.working_bytes - text.len().saturating_mul(64),
                        ..limits
                    },
                    ..builder
                },
                path: String::new(),
            };
            let root = parser.table(document.as_table(), 0)?;
            Ok(parser.builder.finish(root))
        }
    }
}

// A preflight upper allowance covers owned strings, arena nodes, object
// indexes, ancestor paths and temporary builders before their allocation. It
// does not parse JSON or grant malformed data a successful result.
fn json_working_set(bytes: &[u8]) -> usize {
    let mut cursor = 0;
    let mut tokens = 0usize;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'"' => {
                tokens = tokens.saturating_add(1);
                cursor += 1;
                while cursor < bytes.len() {
                    match bytes[cursor] {
                        b'\\' => cursor = cursor.saturating_add(2),
                        b'"' => {
                            cursor += 1;
                            break;
                        }
                        _ => cursor += 1,
                    }
                }
            }
            b'{' | b'[' | b't' | b'f' | b'n' => {
                tokens = tokens.saturating_add(1);
                cursor += 1;
            }
            b'-' | b'0'..=b'9' => {
                tokens = tokens.saturating_add(1);
                cursor += 1;
                while bytes
                    .get(cursor)
                    .is_some_and(|b| matches!(b, b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-'))
                {
                    cursor += 1;
                }
            }
            _ => cursor += 1,
        }
    }
    bytes
        .len()
        .saturating_mul(3)
        .saturating_add(tokens.saturating_mul(512))
}

struct Json {
    text: Arc<str>,
    cursor: usize,
    builder: Builder,
    path: String,
}
impl Json {
    fn error(
        &self,
        kind: DataErrorKind,
        message: impl Into<Text>,
        span: Range<usize>,
    ) -> DataFailure {
        let mut start = span.start.min(self.text.len());
        let mut end = span.end.min(self.text.len());
        while !self.text.is_char_boundary(start) {
            start -= 1;
        }
        while !self.text.is_char_boundary(end) {
            end += 1;
        }
        DataError::new(kind, message)
            .located(self.path.clone(), Some(start..end), Some(self.text.clone()))
            .into()
    }
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.cursor).copied()
    }
    fn space(&mut self) {
        while self
            .peek()
            .is_some_and(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.cursor += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, byte: u8) -> Result<(), DataFailure> {
        if self.take(byte) {
            Ok(())
        } else {
            Err(self.error(
                DataErrorKind::Syntax,
                format!("expected `{}`", byte as char),
                self.cursor..self.cursor + 1,
            ))
        }
    }
    fn string(&mut self) -> Result<Text, DataFailure> {
        let start = self.cursor;
        self.expect(b'"')?;
        loop {
            match self.peek() {
                Some(b'"') => {
                    self.cursor += 1;
                    break;
                }
                Some(b'\\') => {
                    self.cursor += 1;
                    if self.peek().is_none() {
                        break;
                    }
                    self.cursor += 1;
                }
                Some(_) => self.cursor += 1,
                None => {
                    return Err(self.error(
                        DataErrorKind::Syntax,
                        "unterminated JSON string",
                        start..self.cursor,
                    ));
                }
            }
        }
        serde_json::from_str::<String>(&self.text[start..self.cursor])
            .map(Into::into)
            .map_err(|_| {
                self.error(
                    DataErrorKind::Syntax,
                    "invalid JSON string or escape",
                    start..self.cursor,
                )
            })
    }
    fn value(&mut self, depth: usize) -> Result<usize, DataFailure> {
        if depth >= self.builder.limits.depth.min(128) {
            return Err(DataFailure::Limit("data depth limit exceeded"));
        }
        if self.builder.nodes.len() >= self.builder.limits.nodes {
            return Err(DataFailure::Limit("data node limit exceeded"));
        }
        self.space();
        let start = self.cursor;
        let kind = match self.peek() {
            Some(b'"') => NodeKind::String(self.string()?),
            Some(b'n') => {
                self.literal("null")?;
                NodeKind::Null
            }
            Some(b't') => {
                self.literal("true")?;
                NodeKind::Bool(true)
            }
            Some(b'f') => {
                self.literal("false")?;
                NodeKind::Bool(false)
            }
            Some(b'-' | b'0'..=b'9') => self.number()?,
            Some(b'[') => {
                self.cursor += 1;
                self.space();
                let mut values = Vec::new();
                if !self.take(b']') {
                    loop {
                        let old = self.path.len();
                        self.path.push('/');
                        self.path.push_str(&values.len().to_string());
                        values.push(self.value(depth + 1)?);
                        self.path.truncate(old);
                        self.space();
                        if self.take(b']') {
                            break;
                        }
                        self.expect(b',')?;
                        self.space();
                    }
                }
                NodeKind::Array(values)
            }
            Some(b'{') => {
                self.cursor += 1;
                self.space();
                let mut entries = Vec::new();
                let mut index = BTreeMap::new();
                if !self.take(b'}') {
                    loop {
                        let key_start = self.cursor;
                        let key = self.string()?;
                        let old = self.path.len();
                        self.path.push('/');
                        self.path
                            .push_str(&key.replace('~', "~0").replace('/', "~1"));
                        if index.contains_key(&key) {
                            return Err(self.error(
                                DataErrorKind::DuplicateKey,
                                format!("duplicate JSON key `{key}`"),
                                key_start..self.cursor,
                            ));
                        }
                        self.space();
                        self.expect(b':')?;
                        let child = self.value(depth + 1)?;
                        self.path.truncate(old);
                        index.insert(key.clone(), entries.len());
                        entries.push((key, child));
                        self.space();
                        if self.take(b'}') {
                            break;
                        }
                        self.expect(b',')?;
                        self.space();
                    }
                }
                NodeKind::Object { entries, index }
            }
            _ => {
                return Err(self.error(
                    DataErrorKind::Syntax,
                    "expected a JSON value",
                    start..start + 1,
                ));
            }
        };
        self.builder.add(kind, Some(start..self.cursor))
    }
    fn literal(&mut self, literal: &str) -> Result<(), DataFailure> {
        if self.text[self.cursor..].starts_with(literal) {
            self.cursor += literal.len();
            Ok(())
        } else {
            Err(self.error(
                DataErrorKind::Syntax,
                "invalid JSON literal",
                self.cursor..self.cursor + 1,
            ))
        }
    }
    fn number(&mut self) -> Result<NodeKind, DataFailure> {
        let start = self.cursor;
        self.take(b'-');
        if !self.take(b'0') {
            if !self.peek().is_some_and(|b| matches!(b, b'1'..=b'9')) {
                return Err(self.error(
                    DataErrorKind::Syntax,
                    "invalid JSON number",
                    start..self.cursor + 1,
                ));
            }
            self.digits();
        }
        let mut float = false;
        if self.take(b'.') {
            float = true;
            self.required_digits(start)?;
        }
        if self.take(b'e') || self.take(b'E') {
            float = true;
            if !self.take(b'+') {
                self.take(b'-');
            }
            self.required_digits(start)?;
        }
        let value = &self.text[start..self.cursor];
        if float {
            value
                .parse::<f64>()
                .ok()
                .filter(|n| n.is_finite())
                .map(NodeKind::Float)
                .ok_or_else(|| {
                    self.error(
                        DataErrorKind::NumberRange,
                        "JSON float is not finite or is out of range",
                        start..self.cursor,
                    )
                })
        } else {
            value.parse::<i64>().map(NodeKind::Int).map_err(|_| {
                self.error(
                    DataErrorKind::NumberRange,
                    "JSON integer is outside the i64 range",
                    start..self.cursor,
                )
            })
        }
    }
    fn digits(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.cursor += 1;
        }
    }
    fn required_digits(&mut self, start: usize) -> Result<(), DataFailure> {
        if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
            return Err(self.error(
                DataErrorKind::Syntax,
                "expected digits in JSON number",
                start..self.cursor,
            ));
        }
        self.digits();
        Ok(())
    }
}

struct Toml {
    builder: Builder,
    path: String,
}
impl Toml {
    fn check_depth(&self, depth: usize) -> Result<(), DataFailure> {
        if depth >= self.builder.limits.depth.min(128) {
            Err(DataFailure::Limit("data depth limit exceeded"))
        } else {
            Ok(())
        }
    }
    fn error(
        &self,
        kind: DataErrorKind,
        message: impl Into<Text>,
        span: Option<Range<usize>>,
    ) -> DataFailure {
        DataError::new(kind, message)
            .located(self.path.clone(), span, self.builder.text.clone())
            .into()
    }
    fn table(&mut self, table: &toml_edit::Table, depth: usize) -> Result<usize, DataFailure> {
        self.check_depth(depth)?;
        let mut entries = Vec::new();
        let mut index = BTreeMap::new();
        for (key, item) in table.iter() {
            let old = self.path.len();
            self.path.push('/');
            self.path
                .push_str(&key.replace('~', "~0").replace('/', "~1"));
            let child = self.item(item, depth + 1)?;
            self.path.truncate(old);
            index.insert(key.into(), entries.len());
            entries.push((key.into(), child));
        }
        self.builder
            .add(NodeKind::Object { entries, index }, table.span())
    }
    fn item(&mut self, item: &toml_edit::Item, depth: usize) -> Result<usize, DataFailure> {
        self.check_depth(depth)?;
        match item {
            toml_edit::Item::Value(v) => self.value(v, depth),
            toml_edit::Item::Table(t) => self.table(t, depth),
            toml_edit::Item::ArrayOfTables(tables) => {
                let mut values = Vec::new();
                for (i, table) in tables.iter().enumerate() {
                    let old = self.path.len();
                    self.path.push('/');
                    self.path.push_str(&i.to_string());
                    values.push(self.table(table, depth + 1)?);
                    self.path.truncate(old);
                }
                self.builder.add(NodeKind::Array(values), tables.span())
            }
            _ => Err(self.error(
                DataErrorKind::UnsupportedValue,
                "unsupported TOML data value",
                item.span(),
            )),
        }
    }
    fn value(&mut self, value: &toml_edit::Value, depth: usize) -> Result<usize, DataFailure> {
        self.check_depth(depth)?;
        let kind = match value {
            toml_edit::Value::String(v) => NodeKind::String(v.value().as_str().into()),
            toml_edit::Value::Integer(v) => NodeKind::Int(*v.value()),
            toml_edit::Value::Boolean(v) => NodeKind::Bool(*v.value()),
            toml_edit::Value::Float(v) if v.value().is_finite() => NodeKind::Float(*v.value()),
            toml_edit::Value::Float(_) => {
                return Err(self.error(
                    DataErrorKind::NumberRange,
                    "non-finite TOML floats are not supported",
                    value.span(),
                ));
            }
            toml_edit::Value::Datetime(_) => {
                return Err(self.error(
                    DataErrorKind::UnsupportedValue,
                    "unsupported TOML data type: date/time",
                    value.span(),
                ));
            }
            toml_edit::Value::Array(array) => {
                let mut values = Vec::new();
                for (i, value) in array.iter().enumerate() {
                    let old = self.path.len();
                    self.path.push('/');
                    self.path.push_str(&i.to_string());
                    values.push(self.value(value, depth + 1)?);
                    self.path.truncate(old);
                }
                NodeKind::Array(values)
            }
            toml_edit::Value::InlineTable(table) => {
                let mut entries = Vec::new();
                let mut index = BTreeMap::new();
                for (key, value) in table.iter() {
                    let old = self.path.len();
                    self.path.push('/');
                    self.path
                        .push_str(&key.replace('~', "~0").replace('/', "~1"));
                    let child = self.value(value, depth + 1)?;
                    self.path.truncate(old);
                    index.insert(key.into(), entries.len());
                    entries.push((key.into(), child));
                }
                NodeKind::Object { entries, index }
            }
        };
        self.builder.add(kind, value.span())
    }
}
