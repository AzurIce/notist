//! Pure, bounded structured data. Views own no reader, grant or input session.
//! Parsing/projection caches are explicitly supplied by the host input batch.
mod cache;
mod functions;
mod parse;
mod retention;
#[cfg(feature = "serde")]
mod wire;
pub use cache::{DataCache, DataCacheStats, ParsedDataCache};
pub use functions::{DataFunction, definitions};
pub use parse::parse;
pub use retention::DataRetention;

use crate::{
    definitions::Type,
    item::{SourceLocation, Value},
    shared::{Array, Text},
};
use std::{collections::BTreeMap, fmt, ops::Range, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DataFormat {
    Json,
    Toml,
}

/// Hard limits, also part of cache identity. Logical sizes count every selected
/// occurrence, including shared descendants, not the size of a view pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DataLimits {
    pub source_bytes: usize,
    pub nodes: usize,
    pub depth: usize,
    pub value_bytes: usize,
    /// Conservative resident/format-parser working-set bound.
    pub working_bytes: usize,
}
impl Default for DataLimits {
    fn default() -> Self {
        Self {
            source_bytes: 16 * 1024 * 1024,
            nodes: 250_000,
            depth: 64,
            value_bytes: 16 * 1024 * 1024,
            working_bytes: 128 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DataErrorKind {
    NotFound,
    AccessDenied,
    NotFile,
    Io,
    Utf8,
    Syntax,
    NumberRange,
    DuplicateKey,
    UnsupportedValue,
    TypeMismatch,
    MissingKey,
    OutOfRange,
    InvalidQuery,
    UnsupportedTarget,
}
impl DataErrorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::AccessDenied => "access_denied",
            Self::NotFile => "not_file",
            Self::Io => "io",
            Self::Utf8 => "utf8",
            Self::Syntax => "syntax",
            Self::NumberRange => "number_range",
            Self::DuplicateKey => "duplicate_key",
            Self::UnsupportedValue => "unsupported_value",
            Self::TypeMismatch => "type_mismatch",
            Self::MissingKey => "missing_key",
            Self::OutOfRange => "out_of_range",
            Self::InvalidQuery => "invalid_query",
            Self::UnsupportedTarget => "unsupported_target",
        }
    }
}

#[derive(Clone)]
struct DataSource {
    name: String,
    read: SourceLocation,
}

#[derive(Clone)]
pub struct DataError {
    detail: Arc<ErrorDetail>,
    text: Option<Arc<str>>,
    source: Option<Arc<DataSource>>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
struct ErrorDetail {
    kind: DataErrorKind,
    message: Text,
    path: Text,
    expected: Option<Text>,
    actual: Option<Text>,
    span: Option<Range<usize>>,
}
impl DataError {
    pub fn new(kind: DataErrorKind, message: impl Into<Text>) -> Self {
        Self {
            detail: Arc::new(ErrorDetail {
                kind,
                message: message.into(),
                path: "".into(),
                expected: None,
                actual: None,
                span: None,
            }),
            text: None,
            source: None,
        }
    }
    pub fn kind(&self) -> DataErrorKind {
        self.detail.kind
    }
    pub fn message(&self) -> &str {
        &self.detail.message
    }
    /// RFC 6901 pointer, empty for the document root.
    pub fn path(&self) -> &str {
        &self.detail.path
    }
    pub fn expected(&self) -> Option<&str> {
        self.detail.expected.as_deref()
    }
    pub fn actual(&self) -> Option<&str> {
        self.detail.actual.as_deref()
    }
    pub fn logical_size(&self) -> usize {
        32usize
            .saturating_add(self.message().len())
            .saturating_add(self.path().len())
            .saturating_add(self.expected().map_or(0, str::len))
            .saturating_add(self.actual().map_or(0, str::len))
    }
    fn resident_bytes(&self) -> usize {
        self.logical_size()
            .saturating_add(self.text.as_ref().map_or(0, |t| t.len()))
            .saturating_add(128)
    }
    pub fn bind_source(mut self, name: String, read: SourceLocation) -> Self {
        self.source = Some(Arc::new(DataSource { name, read }));
        self
    }
    /// Exact parsed text, never reread from a live resource for diagnostics.
    pub fn diagnostic_source(&self) -> Option<(&str, Arc<str>, Range<usize>, &SourceLocation)> {
        Some((
            &self.source.as_ref()?.name,
            self.text.clone()?,
            self.detail.span.clone().unwrap_or(0..0),
            &self.source.as_ref()?.read,
        ))
    }
    fn located(mut self, path: String, span: Option<Range<usize>>, text: Option<Arc<str>>) -> Self {
        let detail = Arc::make_mut(&mut self.detail);
        detail.path = path.into();
        detail.span = span;
        self.text = text;
        self
    }
}
impl PartialEq for DataError {
    fn eq(&self, other: &Self) -> bool {
        self.kind() == other.kind()
            && self.message() == other.message()
            && self.path() == other.path()
            && self.expected() == other.expected()
            && self.actual() == other.actual()
    }
}
impl Eq for DataError {}
impl fmt::Debug for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let short = |s: &str| {
            let mut value: String = s.chars().take(160).collect();
            if value.len() < s.len() {
                value.push('…');
            }
            value
        };
        f.debug_struct("DataError")
            .field("kind", &self.kind())
            .field("path", &short(self.path()))
            .field("message", &short(self.message()))
            .finish()
    }
}
impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.path().is_empty() {
            write!(f, "{}: ", self.path())?;
        }
        f.write_str(self.message())?;
        if self.source.is_none()
            && let Some(span) = &self.detail.span
        {
            write!(f, " (data bytes {}..{})", span.start, span.end)?;
        }
        Ok(())
    }
}
impl std::error::Error for DataError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataFailure {
    Error(DataError),
    Limit(&'static str),
}
impl From<DataError> for DataFailure {
    fn from(error: DataError) -> Self {
        Self::Error(error)
    }
}
impl fmt::Display for DataFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Error(e) => e.fmt(f),
            Self::Limit(m) => f.write_str(m),
        }
    }
}
impl std::error::Error for DataFailure {}

#[derive(Clone)]
pub struct Data {
    document: Arc<Document>,
    node: usize,
    source: Option<Arc<DataSource>>,
}
struct Document {
    nodes: Vec<Node>,
    text: Option<Arc<str>>,
    resident_bytes: usize,
}
struct Node {
    kind: NodeKind,
    span: Option<Range<usize>>,
    parent: Option<(usize, PathPart)>,
    bytes: usize,
    count: usize,
    depth: usize,
}
enum NodeKind {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Text),
    Array(Vec<usize>),
    Object {
        entries: Vec<(Text, usize)>,
        index: BTreeMap<Text, usize>,
    },
}
#[derive(Clone)]
enum PathPart {
    Key(Text),
    Index(usize),
}
impl NodeKind {
    fn name(&self) -> &'static str {
        match self {
            Self::Null => "Null",
            Self::Bool(_) => "Bool",
            Self::Int(_) => "Int",
            Self::Float(_) => "Float",
            Self::String(_) => "String",
            Self::Array(_) => "Array",
            Self::Object { .. } => "Object",
        }
    }
}
impl Data {
    pub fn kind(&self) -> &'static str {
        self.current().kind.name()
    }
    pub fn logical_size(&self) -> usize {
        self.current().bytes
    }
    pub fn node_count(&self) -> usize {
        self.current().count
    }
    pub fn depth(&self) -> usize {
        self.current().depth
    }
    pub fn shares_document(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.document, &other.document)
    }
    pub fn bind_source(mut self, name: String, read: SourceLocation) -> Self {
        self.source = Some(Arc::new(DataSource { name, read }));
        self
    }
    pub fn path(&self) -> String {
        let mut parts = Vec::new();
        let mut node = self.node;
        while let Some((parent, part)) = &self.document.nodes[node].parent {
            parts.push(part);
            node = *parent;
        }
        let mut result = String::new();
        for part in parts.into_iter().rev() {
            result.push('/');
            match part {
                PathPart::Index(i) => result.push_str(&i.to_string()),
                PathPart::Key(k) => result.push_str(&k.replace('~', "~0").replace('/', "~1")),
            }
        }
        result
    }
    pub fn get(&self, key: &str) -> Result<Option<Self>, DataFailure> {
        match &self.current().kind {
            NodeKind::Object { index, entries } => {
                Ok(index.get(key).map(|i| self.child(entries[*i].1)))
            }
            _ => Err(self.mismatch("Object").into()),
        }
    }
    pub fn require(&self, key: &str) -> Result<Self, DataFailure> {
        self.get(key)?.ok_or_else(|| {
            let mut error = self.error(
                DataErrorKind::MissingKey,
                format!("required key `{key}` is missing"),
            );
            Arc::make_mut(&mut error.detail).path = format!(
                "{}/{}",
                self.path(),
                key.replace('~', "~0").replace('/', "~1")
            )
            .into();
            error.into()
        })
    }
    pub fn index(&self, index: i64) -> Result<Option<Self>, DataFailure> {
        let NodeKind::Array(values) = &self.current().kind else {
            return Err(self.mismatch("Array").into());
        };
        if index < 0 {
            return Err(self
                .error(
                    DataErrorKind::InvalidQuery,
                    "negative data index is not supported",
                )
                .into());
        }
        Ok(usize::try_from(index)
            .ok()
            .and_then(|i| values.get(i))
            .map(|i| self.child(*i)))
    }
    pub fn require_index(&self, index: i64) -> Result<Self, DataFailure> {
        self.index(index)?.ok_or_else(|| {
            let mut error = self.error(
                DataErrorKind::OutOfRange,
                format!("required index {index} is out of range"),
            );
            Arc::make_mut(&mut error.detail).path = format!("{}/{index}", self.path()).into();
            error.into()
        })
    }
    pub fn len(&self) -> Result<usize, DataFailure> {
        match &self.current().kind {
            NodeKind::Array(values) => Ok(values.len()),
            _ => Err(self.mismatch("Array").into()),
        }
    }
    pub fn is_empty(&self) -> Result<bool, DataFailure> {
        self.len().map(|n| n == 0)
    }
    pub fn keys(&self) -> Result<Array, DataFailure> {
        match &self.current().kind {
            NodeKind::Object { entries, .. } => {
                Ok(entries.iter().map(|(k, _)| Value::Str(k.clone())).collect())
            }
            _ => Err(self.mismatch("Object").into()),
        }
    }
    pub fn decode(&self, target: &Type, limits: DataLimits) -> Result<Value, DataFailure> {
        if !decode_target(target) {
            return Err(self
                .error(
                    DataErrorKind::UnsupportedTarget,
                    format!("unsupported Data decode target: {target}"),
                )
                .into());
        }
        self.check_limits(limits)?;
        let mut budget = DecodeBudget {
            bytes: limits.value_bytes,
            nodes: limits.nodes,
        };
        let value = self.decode_inner(target, limits.depth.min(128), &mut budget)?;
        if value.logical_size() > limits.value_bytes {
            return Err(DataFailure::Limit(
                "data projection value size limit exceeded",
            ));
        }
        if value.resident_size() > limits.working_bytes {
            return Err(DataFailure::Limit(
                "data projection working-set limit exceeded",
            ));
        }
        Ok(value)
    }
    fn decode_inner(
        &self,
        target: &Type,
        remaining: usize,
        budget: &mut DecodeBudget,
    ) -> Result<Value, DataFailure> {
        if remaining == 0 {
            return Err(DataFailure::Limit("data projection depth limit exceeded"));
        }
        let (bytes, nodes) = match (target, &self.current().kind) {
            (Type::Data, _) => (self.logical_size(), self.node_count()),
            (Type::String, NodeKind::String(s)) => (s.len(), 1),
            (Type::Array(Some(_)), NodeKind::Array(v)) => (v.len().saturating_mul(8), 1),
            _ => (8, 1),
        };
        budget.charge(bytes, nodes)?;
        Ok(match (target, &self.current().kind) {
            (Type::Data, _) => Value::Data(self.clone()),
            (Type::Unit, NodeKind::Null) => Value::Unit,
            (Type::Bool, NodeKind::Bool(v)) => Value::Bool(*v),
            (Type::Int, NodeKind::Int(v)) => Value::Int(*v),
            (Type::Float, NodeKind::Float(v)) => Value::Float(*v),
            (Type::String, NodeKind::String(v)) => Value::Str(v.clone()),
            (Type::Option(_), NodeKind::Null) => Value::None,
            (Type::Option(t), _) => {
                Value::Some(Box::new(self.decode_inner(t, remaining - 1, budget)?))
            }
            (Type::Array(Some(t)), NodeKind::Array(values)) => Value::Array(
                values
                    .iter()
                    .map(|i| self.child(*i).decode_inner(t, remaining - 1, budget))
                    .collect::<Result<Array, _>>()?,
            ),
            _ => return Err(self.mismatch(&target.to_string()).into()),
        })
    }
    fn check_limits(&self, limits: DataLimits) -> Result<(), DataFailure> {
        if self.node_count() > limits.nodes {
            return Err(DataFailure::Limit("data node limit exceeded"));
        }
        if self.depth() > limits.depth.min(128) {
            return Err(DataFailure::Limit("data depth limit exceeded"));
        }
        if self.logical_size() > limits.value_bytes {
            return Err(DataFailure::Limit("data logical value size limit exceeded"));
        }
        Ok(())
    }
    fn current(&self) -> &Node {
        &self.document.nodes[self.node]
    }
    fn child(&self, node: usize) -> Self {
        Self {
            document: self.document.clone(),
            node,
            source: self.source.clone(),
        }
    }
    fn error(&self, kind: DataErrorKind, message: impl Into<Text>) -> DataError {
        let mut error = DataError::new(kind, message).located(
            self.path(),
            self.current().span.clone(),
            self.document.text.clone(),
        );
        error.source = self.source.clone();
        error
    }
    fn mismatch(&self, expected: &str) -> DataError {
        let mut error = self.error(
            DataErrorKind::TypeMismatch,
            format!("expected {expected}, got {}", self.kind()),
        );
        let detail = Arc::make_mut(&mut error.detail);
        detail.expected = Some(expected.into());
        detail.actual = Some(self.kind().into());
        error
    }
    fn unbound(&self) -> Self {
        Self {
            source: None,
            ..self.clone()
        }
    }
}
struct DecodeBudget {
    bytes: usize,
    nodes: usize,
}
impl DecodeBudget {
    fn charge(&mut self, bytes: usize, nodes: usize) -> Result<(), DataFailure> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or(DataFailure::Limit(
            "data projection value size limit exceeded",
        ))?;
        self.nodes = self
            .nodes
            .checked_sub(nodes)
            .ok_or(DataFailure::Limit("data projection node limit exceeded"))?;
        Ok(())
    }
}
/// Validate the complete target before observing values (including null/empty).
pub fn decode_target(target: &Type) -> bool {
    fn allowed(t: &Type, depth: usize) -> bool {
        if depth == 0 {
            return false;
        }
        match t {
            Type::Data | Type::Unit | Type::Bool | Type::Int | Type::Float | Type::String => true,
            Type::Array(Some(t)) | Type::Option(t) => allowed(t, depth - 1),
            _ => false,
        }
    }
    allowed(target, 64)
}
impl fmt::Debug for Data {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Data")
            .field("kind", &self.kind())
            .field("nodes", &self.node_count())
            .field("bytes", &self.logical_size())
            .finish()
    }
}
impl PartialEq for Data {
    fn eq(&self, other: &Self) -> bool {
        fn eq(a: &Data, b: &Data) -> bool {
            match (&a.current().kind, &b.current().kind) {
                (NodeKind::Null, NodeKind::Null) => true,
                (NodeKind::Bool(a), NodeKind::Bool(b)) => a == b,
                (NodeKind::Int(a), NodeKind::Int(b)) => a == b,
                (NodeKind::Float(a), NodeKind::Float(b)) => a == b,
                (NodeKind::String(a), NodeKind::String(b)) => a == b,
                (NodeKind::Array(x), NodeKind::Array(y)) => {
                    x.len() == y.len()
                        && x.iter().zip(y).all(|(x, y)| eq(&a.child(*x), &b.child(*y)))
                }
                (NodeKind::Object { entries: x, .. }, NodeKind::Object { entries: y, .. }) => {
                    x.len() == y.len()
                        && x.iter()
                            .zip(y)
                            .all(|((k, x), (l, y))| k == l && eq(&a.child(*x), &b.child(*y)))
                }
                _ => false,
            }
        }
        eq(self, other)
    }
}

struct Builder {
    nodes: Vec<Node>,
    text: Option<Arc<str>>,
    limits: DataLimits,
    bytes: usize,
}
impl Builder {
    fn add(&mut self, kind: NodeKind, span: Option<Range<usize>>) -> Result<usize, DataFailure> {
        if self.nodes.len() >= self.limits.nodes {
            return Err(DataFailure::Limit("data node limit exceeded"));
        }
        let mut bytes = 8usize;
        let mut count = 1usize;
        let mut depth = 1usize;
        let mut children = Vec::new();
        match &kind {
            NodeKind::String(s) => bytes = bytes.saturating_add(s.len()),
            NodeKind::Array(values) => {
                bytes = bytes.saturating_add(values.len().saturating_mul(8));
                children.extend(
                    values
                        .iter()
                        .enumerate()
                        .map(|(i, n)| (*n, PathPart::Index(i))),
                );
            }
            NodeKind::Object { entries, .. } => {
                for (key, node) in entries {
                    bytes = bytes.saturating_add(key.len() + 8);
                    children.push((*node, PathPart::Key(key.clone())));
                }
            }
            _ => {}
        }
        let index_bytes = match &kind {
            NodeKind::Object { entries, .. } => entries.len().saturating_mul(128),
            _ => 0,
        };
        self.bytes = self
            .bytes
            .saturating_add(bytes)
            .saturating_add(std::mem::size_of::<Node>())
            .saturating_add(64)
            .saturating_add(index_bytes);
        if self.bytes > self.limits.working_bytes {
            return Err(DataFailure::Limit("data parser working-set limit exceeded"));
        }
        for (child, _) in &children {
            let n = &self.nodes[*child];
            bytes = bytes.saturating_add(n.bytes);
            count = count.saturating_add(n.count);
            depth = depth.max(n.depth + 1);
        }
        if bytes > self.limits.value_bytes {
            return Err(DataFailure::Limit("data logical value size limit exceeded"));
        }
        if depth > self.limits.depth.min(128) {
            return Err(DataFailure::Limit("data depth limit exceeded"));
        }
        let id = self.nodes.len();
        for (child, part) in children {
            self.nodes[child].parent = Some((id, part));
        }
        self.nodes.push(Node {
            kind,
            span,
            parent: None,
            bytes,
            count,
            depth,
        });
        Ok(id)
    }
    fn finish(self, node: usize) -> Data {
        Data {
            document: Arc::new(Document {
                nodes: self.nodes,
                text: self.text,
                resident_bytes: self.bytes,
            }),
            node,
            source: None,
        }
    }
}

// Cache entries never own a bound source view. Rebinding ordinary scalar arrays
// is O(1); arrays of Data materialize only node handles, not their subtrees.
fn bind_value(value: &Value, source: &Option<Arc<DataSource>>) -> Value {
    if !value.contains_data_views() {
        return value.clone();
    }
    match value {
        Value::Data(data) => Value::Data(Data {
            source: source.clone(),
            ..data.clone()
        }),
        Value::DataError(error) => Value::DataError(DataError {
            source: source.clone(),
            ..error.clone()
        }),
        Value::Array(values) => {
            Value::Array(values.iter().map(|v| bind_value(v, source)).collect())
        }
        Value::Some(v) => Value::Some(Box::new(bind_value(v, source))),
        Value::Ok(v) => Value::Ok(Box::new(bind_value(v, source))),
        Value::Err(v) => Value::Err(Box::new(bind_value(v, source))),
        _ => value.clone(),
    }
}
