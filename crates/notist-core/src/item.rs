use rowan::TextRange;

use crate::builtins;
use crate::definitions::FunctionId;

/// A registered call's public contract snapshot, independent of its module files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionCtor {
    pub id: crate::definitions::DefId,
    pub accepts: builtins::Accepts,
    pub level: builtins::Level,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Ctor {
    Doc,
    Paragraph,
    Heading,
    Text,
    RawInline,
    Strong,
    Emph,
    Strike,
    Underline,
    Math,
    Link,
    /// An unresolved resource embedding. `target` identifies the resource;
    /// `description` and optional `title` describe it. Rendering determines
    /// the resource type and presentation; the payload lives in fields.
    Embed,
    /// A table of rows. `align` stores the column alignment array.
    Table,
    /// A table row of cells. `header: true` marks a header row.
    TableRow,
    /// A table cell containing ordinary document content.
    TableCell,
    List,
    ListItem,
    /// A block-level callout. Markdown quotations carry `kind: "quote"`.
    Callout,
    /// A thematic break separating content, with no children.
    Divider,
    /// Transparent identity node: renders as its children; carries attrs
    /// and structure without adding semantics. Produced by explicit `#node`.
    Node,
    /// A document section: produced by `sectionize`, the identity carrier of
    /// its range; the span covers the whole extent and the heading stays the
    /// first child. An annotation written before the heading attaches to the
    /// section after shaping. The source spelling `#section[..]` resolves to
    /// the same constructor.
    Section,
    /// A typed content-value mount, distinct from a named function call.
    /// Evaluation retains this group around the resulting forest, including
    /// empty and attribute-free forests, so annotations keep their own target.
    Interpolation,
    /// A transparent result group, retained for explicit or annotated mounts.
    Group,
    /// A recovered content failure. The message is result data, not evaluator state.
    Error(String),
    /// A legitimate registered function, distinct from unknown-name recovery.
    #[cfg_attr(feature = "serde", serde(skip))]
    Extension(ExtensionCtor),
    /// Recovery representation of an unresolved name or unsupported
    /// expression, accompanied by a type diagnostic. Not a registered
    /// constructor.
    Custom(String),
}

impl Ctor {
    /// The source-level name of a builtin constructor (`Doc` and custom
    /// constructors have none).
    pub(crate) fn source_name(&self) -> Option<&'static str> {
        Some(match self {
            Ctor::Doc
            | Ctor::Interpolation
            | Ctor::Group
            | Ctor::Error(_)
            | Ctor::Custom(_)
            | Ctor::Extension(_) => return None,
            Ctor::Paragraph => "paragraph",
            Ctor::Heading => "heading",
            Ctor::Text => "text",
            Ctor::RawInline => "raw",
            Ctor::Strong => "strong",
            Ctor::Emph => "emph",
            Ctor::Strike => "strike",
            Ctor::Underline => "underline",
            Ctor::Math => "math",
            Ctor::Link => "link",
            Ctor::Embed => "embed",
            Ctor::Table => "table",
            Ctor::TableRow => "row",
            Ctor::TableCell => "cell",
            Ctor::List => "list",
            Ctor::ListItem => "item",
            Ctor::Callout => "callout",
            Ctor::Divider => "divider",
            Ctor::Node => "node",
            Ctor::Section => "section",
        })
    }

    /// The builtin constructor for a source-level name.
    pub fn from_name(name: &str) -> Option<Ctor> {
        Some(match name {
            "paragraph" => Ctor::Paragraph,
            "heading" => Ctor::Heading,
            "text" => Ctor::Text,
            "raw" => Ctor::RawInline,
            "strong" => Ctor::Strong,
            "emph" => Ctor::Emph,
            "strike" => Ctor::Strike,
            "underline" => Ctor::Underline,
            "math" => Ctor::Math,
            "link" => Ctor::Link,
            "embed" => Ctor::Embed,
            "table" => Ctor::Table,
            "row" => Ctor::TableRow,
            "cell" => Ctor::TableCell,
            "list" => Ctor::List,
            "item" => Ctor::ListItem,
            "callout" => Ctor::Callout,
            "divider" => Ctor::Divider,
            "node" => Ctor::Node,
            "section" => Ctor::Section,
            _ => return None,
        })
    }

    /// The element level of a builtin constructor, if statically known.
    /// `Doc` and custom constructors return `None` (derive it structurally).
    pub fn level(&self) -> Option<builtins::Level> {
        if let Self::Extension(function) = self {
            return Some(function.level);
        }
        self.source_name()
            .and_then(builtins::builtin_signature)
            .map(|s| s.level)
    }

    /// What a builtin constructor's children mount accepts.
    pub fn accepts(&self) -> Option<builtins::Accepts> {
        if let Self::Extension(function) = self {
            return Some(function.accepts);
        }
        self.source_name()
            .and_then(builtins::builtin_signature)
            .map(|s| s.accepts)
    }

    pub fn name(
        &self,
        definitions: &crate::definition_identity::Definitions,
    ) -> Result<std::borrow::Cow<'_, str>, crate::definition_identity::DefinitionError> {
        if let Self::Extension(function) = self {
            return Ok(definitions.function(function.id)?.to_string().into());
        }
        Ok(std::borrow::Cow::Borrowed(match self {
            Ctor::Doc => "Doc",
            Ctor::Paragraph => "Paragraph",
            Ctor::Heading => "Heading",
            Ctor::Text => "Text",
            Ctor::RawInline => "RawInline",
            Ctor::Strong => "Strong",
            Ctor::Emph => "Emph",
            Ctor::Strike => "Strike",
            Ctor::Underline => "Underline",
            Ctor::Math => "Math",
            Ctor::Link => "Link",
            Ctor::Embed => "Embed",
            Ctor::Table => "Table",
            Ctor::TableRow => "TableRow",
            Ctor::TableCell => "TableCell",
            Ctor::List => "List",
            Ctor::ListItem => "ListItem",
            Ctor::Callout => "Callout",
            Ctor::Divider => "Divider",
            Ctor::Node => "Node",
            Ctor::Section => "Section",
            Ctor::Interpolation => "Interpolation",
            Ctor::Group => "Group",
            Ctor::Error(_) => "Error",
            Ctor::Custom(name) => name,
            Ctor::Extension(_) => unreachable!(),
        }))
    }

    /// Canonical identity for both builtin and registered external calls.
    pub fn function_id(&self) -> Option<crate::definitions::DefId> {
        match self {
            Self::Extension(function) => Some(function.id),
            _ => self
                .source_name()
                .and_then(|name| builtins::registry().function("notist", name))
                .map(|definition| definition.id.def_id()),
        }
    }
    /// A builtin constructor carries its fixed symbolic identity intrinsically.
    /// Dynamically resolved extension IDs always require the supplied owners.
    pub fn function_name<'a>(
        &self,
        definitions: &'a crate::definition_identity::Definitions,
    ) -> Result<Option<&'a FunctionId>, crate::definition_identity::DefinitionError> {
        match self {
            Self::Extension(extension) => definitions.function(extension.id).map(Some),
            _ => Ok(self
                .source_name()
                .and_then(|name| builtins::registry().function("notist", name))
                .map(|definition| &definition.id)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(#[cfg_attr(feature = "serde", serde(with = "float_serde"))] f64),
    Str(crate::shared::Text),
    Path(crate::resource::ResourcePath),
    Array(crate::shared::Array),
    Dict(Dict),
    Some(Box<Value>),
    None,
    Ok(Box<Value>),
    Err(Box<Value>),
    Data(crate::data::Data),
    DataError(crate::data::DataError),
}

pub use crate::shared::Dict;

impl Value {
    /// Resource identities must not be implicitly exposed as HTML or wire data.
    pub fn contains_path(&self) -> bool {
        self.metrics().path
    }
    pub fn contains_data_views(&self) -> bool {
        self.metrics().data
    }
    pub fn logical_size(&self) -> usize {
        self.metrics().bytes
    }
    /// Conservative owned container/string storage, excluding Data backings
    /// accounted once per document by input caches and DataRetention.
    pub fn resident_size(&self) -> usize {
        self.metrics().resident
    }
    pub(crate) fn metrics(&self) -> crate::shared::Metrics {
        use crate::shared::Metrics;
        match self {
            Self::Data(data) => Metrics {
                bytes: data.logical_size(),
                path: false,
                data: true,
                resident: 0,
            },
            Self::DataError(error) => Metrics {
                bytes: error.logical_size(),
                path: false,
                data: true,
                resident: error.logical_size().saturating_add(128),
            },
            Self::Str(text) => Metrics {
                bytes: text.len(),
                resident: text.len().saturating_add(16),
                ..Default::default()
            },
            Self::Path(path) => Metrics {
                bytes: path.root().len().saturating_add(path.path().len()),
                path: true,
                data: false,
                resident: path
                    .root()
                    .len()
                    .saturating_add(path.path().len())
                    .saturating_add(32),
            },
            Self::Array(values) => values.metrics(),
            Self::Dict(dict) => dict.metrics(),
            Self::Some(value) | Self::Ok(value) | Self::Err(value) => {
                let mut metrics = value.metrics();
                metrics.bytes = metrics.bytes.saturating_add(8);
                metrics.resident = metrics
                    .resident
                    .saturating_add(std::mem::size_of::<Value>());
                metrics
            }
            _ => Metrics {
                bytes: 8,
                ..Default::default()
            },
        }
    }
}

// Keep existing finite-float JSON while allowing every f64 to round-trip through
// serializers (including JSON, whose number grammar excludes NaN/infinity).
#[cfg(feature = "serde")]
pub(crate) mod float_serde {
    use serde::{Deserialize, Serialize};
    pub fn serialize<S: serde::Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        if !serializer.is_human_readable() || value.is_finite() {
            return serializer.serialize_f64(*value);
        }
        #[derive(Serialize)]
        struct Bits {
            bits: String,
        }
        Bits {
            bits: format!("{:016x}", value.to_bits()),
        }
        .serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        if !deserializer.is_human_readable() {
            return f64::deserialize(deserializer);
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Number(f64),
            Bits { bits: String },
        }
        match Stored::deserialize(deserializer)? {
            Stored::Number(value) => Ok(value),
            Stored::Bits { bits } => {
                if bits.len() != 16 {
                    return Err(serde::de::Error::custom("expected sixteen f64 hex digits"));
                }
                u64::from_str_radix(&bits, 16)
                    .map(f64::from_bits)
                    .map_err(serde::de::Error::custom)
            }
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Unit => write!(f, "()"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Str(s) => write!(f, "{s:?}"),
            Value::Path(path) => write!(f, "path({:?}, {:?})", path.root(), path.path()),
            Value::Data(data) => write!(f, "{data:?}"),
            Value::DataError(error) => write!(f, "{error:?}"),
            Value::Array(items) => {
                write!(f, "(")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{item}")?;
                }
                write!(f, ")")
            }
            Value::Dict(dict) => {
                write!(f, "(")?;
                for (i, (key, value)) in dict.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{key:?}: {value}")?;
                }
                write!(f, ")")
            }
            Value::Some(value) => write!(f, "some({value})"),
            Value::None => f.write_str("none"),
            Value::Ok(value) => write!(f, "ok({value})"),
            Value::Err(value) => write!(f, "err({value})"),
        }
    }
}

/// Source locations remain meaningful across Code definition expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    /// Interpreted only in the source snapshot owning this tree.
    pub source: crate::source::SourceId,
    pub start: u32,
    pub end: u32,
    pub expansion: crate::source_chain::SourceChain,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    pub source: crate::source::SourceId,
    pub start: u32,
    pub end: u32,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub content: NodeContent,
    pub attrs: Dict,
    pub origin: Option<Origin>,
    /// Resolved content category. Document analysis always stores Block or Inline.
    pub level: builtins::Level,
    pub span: TextRange,
}

/// A content tree can contain expressions before evaluation. Complete output
/// contains only `Item` variants; transparent groups are ordinary content.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeContent {
    Item(ContentItem),
    Expr(crate::shared::Shared<crate::evaluation::TypedExpr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContentItem {
    pub ctor: Ctor,
    pub fields: Dict,
    pub children: crate::shared::Shared<Vec<Node>>,
    /// Concrete native/component parameters, not a source call or execution state.
    pub instance: Option<crate::shared::Shared<crate::definitions::ResolvedInstance>>,
}

impl crate::shared::Shared<Vec<Node>> {
    /// Per-thread empty child storage. Mutable access still detaches normally.
    /// Keep this concrete: other Shared values retain their owned construction.
    pub fn empty_nodes() -> Self {
        // Avoid a single process-wide refcount on parallel construction paths.
        // Values moved to another thread retain their ordinary Arc ownership.
        thread_local! {
            static EMPTY: crate::shared::Shared<Vec<Node>> =
                crate::shared::Shared::new(Vec::new());
        }
        // Construction can also happen in another thread-local's destructor.
        EMPTY
            .try_with(Clone::clone)
            .unwrap_or_else(|_| Self::new(Vec::new()))
    }

    /// Reuse empty child storage without discarding a caller's reserved capacity.
    pub fn from_nodes(nodes: Vec<Node>) -> Self {
        if nodes.capacity() == 0 {
            Self::empty_nodes()
        } else {
            Self::new(nodes)
        }
    }
}

/// Transitional spelling for public content-tree callers.
pub type Item = Node;

impl Node {
    /// Content constructor, or a tooling projection of a checked call's identity.
    /// Non-call expressions project to `Interpolation`; inspect `content` to
    /// distinguish expressions from constructed content.
    pub fn ctor(&self) -> Ctor {
        if let Some(item) = self.as_item() {
            return item.ctor.clone();
        }
        match self.as_expr().map(|e| &e.kind) {
            Some(crate::evaluation::ExprKind::Call(call)) => {
                let def = call.definition();
                if def.id.module == "notist" {
                    Ctor::from_name(&def.id.name).unwrap_or(Ctor::Interpolation)
                } else {
                    Ctor::Extension(ExtensionCtor {
                        id: def.id.def_id(),
                        accepts: def.children,
                        level: self.level,
                    })
                }
            }
            _ => Ctor::Interpolation,
        }
    }
    /// Concrete field, or a known checked-call argument (without evaluating it).
    pub fn field(&self, name: &str) -> Option<&Value> {
        if let Some(item) = self.as_item() {
            return item.fields.get(name);
        }
        let crate::evaluation::ExprKind::Call(call) = &self.as_expr()?.kind else {
            return None;
        };
        if name == "level"
            && call.definition().id.module == "notist"
            && call.definition().id.name == "heading"
        {
            if let Some(crate::definitions::ComptimeValue::Data(value)) =
                call.instance.arguments.first()
            {
                return Some(value);
            }
        }
        let (_, arg) = call.arguments().find(|(key, _)| *key == name)?;
        match arg.known.as_ref() {
            Some(crate::definitions::ComptimeValue::Data(value)) => {
                if call.definition().parameters.iter().any(|p| {
                    p.name == name && p.mode == crate::definitions::ParameterMode::Optional
                }) && matches!(arg.ty, Some(crate::definitions::Type::Option(_)))
                {
                    match value {
                        Value::Some(value) => Some(value),
                        Value::None => None,
                        _ => Some(value),
                    }
                } else {
                    Some(value)
                }
            }
            _ => None,
        }
    }
    /// Copy-on-write snapshot of concrete fields or known call arguments.
    pub fn fields(&self) -> Dict {
        if let Some(item) = self.as_item() {
            return item.fields.clone();
        }
        let Some(crate::evaluation::TypedExpr {
            kind: crate::evaluation::ExprKind::Call(call),
            ..
        }) = self.as_expr()
        else {
            return Dict::default();
        };
        let mut fields = Dict::default();
        for (name, _) in call.arguments() {
            if let Some(value) = self.field(name) {
                fields.insert(name, value.clone());
            }
        }
        if let Some(value) = self.field("level") {
            fields.insert("level", value.clone());
        }
        fields
    }
    /// Content children or literal call children. This is a content-tree view,
    /// not a traversal of all subexpressions; use `as_expr` for computation IR.
    pub fn children(&self) -> &[Node] {
        if let Some(item) = self.as_item() {
            return &item.children;
        }
        fn content(expr: &crate::evaluation::TypedExpr) -> &[Node] {
            match &expr.kind {
                crate::evaluation::ExprKind::Content(nodes) => nodes,
                crate::evaluation::ExprKind::Call(call) => {
                    call.children.as_deref().map(content).unwrap_or(&[])
                }
                _ => &[],
            }
        }
        self.as_expr().map(content).unwrap_or(&[])
    }
    pub fn children_mut(&mut self) -> Option<&mut Vec<Node>> {
        fn content(expr: &mut crate::evaluation::TypedExpr) -> Option<&mut Vec<Node>> {
            match &mut expr.kind {
                crate::evaluation::ExprKind::Content(nodes) => Some(nodes),
                crate::evaluation::ExprKind::Call(call) => {
                    call.children.as_deref_mut().and_then(content)
                }
                _ => None,
            }
        }
        match &mut self.content {
            NodeContent::Item(item) => Some(&mut item.children),
            NodeContent::Expr(expr) => content(expr),
        }
    }
    pub fn instance(&self) -> Option<&crate::definitions::ResolvedInstance> {
        match &self.content {
            NodeContent::Item(item) => item.instance.as_deref(),
            NodeContent::Expr(expr) => match &expr.kind {
                crate::evaluation::ExprKind::Call(call) => Some(&call.instance),
                _ => None,
            },
        }
    }
    pub fn is_error(&self) -> bool {
        matches!(
            &self.content,
            NodeContent::Item(ContentItem {
                ctor: Ctor::Error(_),
                ..
            })
        )
    }
    pub fn as_item(&self) -> Option<&ContentItem> {
        match &self.content {
            NodeContent::Item(item) => Some(item),
            _ => None,
        }
    }
    pub fn as_item_mut(&mut self) -> Option<&mut ContentItem> {
        match &mut self.content {
            NodeContent::Item(item) => Some(item),
            _ => None,
        }
    }
    pub fn as_expr(&self) -> Option<&crate::evaluation::TypedExpr> {
        match &self.content {
            NodeContent::Expr(expr) => Some(expr),
            _ => None,
        }
    }
    pub fn as_expr_mut(&mut self) -> Option<&mut crate::evaluation::TypedExpr> {
        match &mut self.content {
            NodeContent::Expr(expr) => Some(expr),
            _ => None,
        }
    }
    pub fn call(&self) -> Option<&crate::evaluation::Invocation> {
        match &self.as_expr()?.kind {
            crate::evaluation::ExprKind::Call(call) => Some(call),
            _ => None,
        }
    }
    pub fn call_mut(&mut self) -> Option<&mut crate::evaluation::Invocation> {
        match &mut self.as_expr_mut()?.kind {
            crate::evaluation::ExprKind::Call(call) => Some(call),
            _ => None,
        }
    }
    pub fn expression(expr: crate::evaluation::TypedExpr) -> Self {
        let mut node = Self::new(Ctor::Interpolation, expr.span);
        node.origin = Some(Origin {
            source: expr.source,
            start: expr.span.start().into(),
            end: expr.span.end().into(),
            expansion: Default::default(),
        });
        node.level = expr
            .ty
            .as_ref()
            .and_then(crate::definitions::Type::level)
            .unwrap_or(builtins::Level::Inline);
        node.content = NodeContent::Expr(expr.into());
        node
    }
    /// Whether this is ordinary transparent group content.
    pub fn is_group(&self) -> bool {
        self.as_item().is_some_and(|item| item.ctor == Ctor::Group)
    }

    /// Checked source-call identity before evaluation, actual constructor
    /// identity afterwards; absent on documents, groups and recovery nodes.
    pub fn function_id(&self) -> Option<crate::definitions::DefId> {
        self.ctor().function_id()
    }
    pub fn new(ctor: Ctor, span: TextRange) -> Self {
        Self {
            level: ctor
                .level()
                .filter(|level| *level != builtins::Level::Inherit)
                .unwrap_or(if ctor == Ctor::Doc {
                    builtins::Level::Block
                } else {
                    builtins::Level::Inline
                }),
            content: NodeContent::Item(ContentItem {
                ctor,
                fields: Dict::default(),
                children: crate::shared::Shared::empty_nodes(),
                instance: None,
            }),
            origin: None,
            attrs: Dict::default(),
            span,
        }
    }

    pub fn text(text: String, span: TextRange) -> Self {
        Item::new(Ctor::Text, span).with_field("text", Value::Str(text.into()))
    }

    pub fn with_field(mut self, key: impl AsRef<str>, value: Value) -> Self {
        self.as_item_mut()
            .expect("content constructor")
            .fields
            .insert(key, value);
        self.infer_level();
        self
    }

    pub fn with_children(mut self, children: Vec<Item>) -> Self {
        self.as_item_mut().expect("content constructor").children =
            crate::shared::Shared::from_nodes(children);
        self.infer_level();
        self
    }

    fn infer_level(&mut self) {
        use builtins::Level;
        self.level = match &self.ctor() {
            Ctor::Extension(function) => function.level,
            Ctor::Doc => Level::Block,
            Ctor::Node | Ctor::Group | Ctor::Interpolation | Ctor::Error(_) | Ctor::Custom(_) => {
                if self
                    .children()
                    .iter()
                    .any(|child| child.level == Level::Block)
                {
                    Level::Block
                } else {
                    Level::Inline
                }
            }
            _ => self
                .ctor()
                .source_name()
                .and_then(|name| builtins::registry().function("notist", name))
                .map(|definition| definition.returns.level().unwrap_or(Level::Inline))
                .unwrap_or(Level::Inline),
        };
    }

    /// Depth-first, pre-order iteration over this item and all its descendants.
    /// Traversal storage grows with depth, not the number of siblings; shallow
    /// trees use inline storage without allocating a traversal stack.
    #[inline]
    pub fn descendants(&self) -> impl Iterator<Item = &Item> {
        Descendants::new(self)
    }

    /// The first descendant (self included) matching `pred`.
    pub fn find(&self, pred: impl Fn(&Item) -> bool) -> Option<&Item> {
        self.descendants().find(|item| pred(item))
    }
}

/// Keep one child cursor per active level rather than pushing every sibling.
/// Deep trees spill onto the heap instead of recursing on the call stack.
struct Descendants<'a> {
    current: std::slice::Iter<'a, Item>,
    stack: smallvec::SmallVec<[std::slice::Iter<'a, Item>; 8]>,
}

impl<'a> Descendants<'a> {
    #[inline]
    fn new(root: &'a Item) -> Self {
        Self {
            current: std::slice::from_ref(root).iter(),
            stack: smallvec::SmallVec::new(),
        }
    }
}

impl<'a> Iterator for Descendants<'a> {
    type Item = &'a Item;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(item) = self.current.next() {
                let children = item.children();
                if !children.is_empty() {
                    let remaining = std::mem::replace(&mut self.current, children.iter());
                    // A single-child chain needs no saved parent cursor.
                    if !remaining.as_slice().is_empty() {
                        self.stack.push(remaining);
                    }
                }
                return Some(item);
            }
            self.current = self.stack.pop()?;
        }
    }
}

#[cfg(test)]
mod traversal_tests {
    use super::*;

    fn node(children: Vec<Item>) -> Item {
        Item::new(Ctor::Node, TextRange::empty(0.into())).with_children(children)
    }

    #[test]
    fn descendants_preserves_preorder_and_reference_identity() {
        let root = node(vec![node(vec![node(vec![]), node(vec![])]), node(vec![])]);
        let first = &root.children()[0];
        let expected = [
            &root,
            first,
            &first.children()[0],
            &first.children()[1],
            &root.children()[1],
        ];
        let actual: Vec<_> = root.descendants().collect();
        assert_eq!(actual.len(), expected.len());
        for (actual, expected) in actual.into_iter().zip(expected) {
            assert!(std::ptr::eq(actual, expected));
        }
        assert!(std::ptr::eq(root.find(|_| true).unwrap(), &root));
        assert!(root.find(|_| false).is_none());
    }

    #[test]
    fn wide_tree_keeps_inline_cursors_and_original_sibling_order() {
        let root = node((0..4096).map(|_| node(vec![])).collect());
        let mut descendants = Descendants::new(&root);
        assert!(std::ptr::eq(descendants.next().unwrap(), &root));
        for child in root.children() {
            assert!(std::ptr::eq(descendants.next().unwrap(), child));
            assert!(!descendants.stack.spilled());
            assert!(descendants.stack.is_empty());
        }
        assert!(descendants.next().is_none());
        assert!(descendants.next().is_none());
    }

    #[test]
    fn deep_tree_spills_without_recursive_traversal() {
        let depth = 4096;
        let mut root = node(vec![]);
        for _ in 0..depth {
            root = node(vec![root, node(vec![])]);
        }
        let mut descendants = Descendants::new(&root);
        let mut count = 0;
        let mut spilled = false;
        while descendants.next().is_some() {
            count += 1;
            spilled |= descendants.stack.spilled();
        }
        drop(descendants);
        // Teardown is iterative too: recursive Node destruction is a separate
        // concern, and must not overflow the test thread's call stack.
        let mut pending = vec![root];
        while let Some(mut item) = pending.pop() {
            pending.extend(std::mem::take(item.children_mut().unwrap()));
        }
        assert_eq!(count, depth * 2 + 1);
        assert!(spilled);
    }

    #[test]
    fn leaf_is_included_once_and_exhaustion_is_stable() {
        let root = node(vec![]);
        let mut descendants = Descendants::new(&root);
        assert!(!descendants.stack.spilled());
        assert!(std::ptr::eq(descendants.next().unwrap(), &root));
        assert!(descendants.next().is_none());
        assert!(descendants.next().is_none());
        assert!(!descendants.stack.spilled());
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    #[test]
    fn empty_children_share_storage_and_mutation_detaches() {
        use crate::shared::Shared;
        assert_eq!(
            std::mem::size_of::<Shared<Vec<Node>>>(),
            std::mem::size_of::<usize>()
        );
        #[cfg(target_pointer_width = "64")]
        assert_eq!(std::mem::size_of::<Node>(), 104);
        let mut original = Item::new(Ctor::Node, Default::default());
        let empty = Shared::empty_nodes();
        assert!(original.as_item().unwrap().children.shares_storage(&empty));
        let mut changed = original.clone().with_children(Vec::new());
        assert!(changed.as_item().unwrap().children.shares_storage(&empty));
        changed
            .children_mut()
            .unwrap()
            .push(Item::new(Ctor::Divider, Default::default()));
        assert!(original.children().is_empty());
        assert!(!changed.as_item().unwrap().children.shares_storage(&empty));
        original.children_mut().unwrap().reserve_exact(2);
        assert!(empty.is_empty());
        assert_eq!(empty.capacity(), 0);
        assert!(original.children().is_empty());
        assert_eq!(original.as_item().unwrap().children.capacity(), 2);

        let reserved = Shared::from_nodes(Vec::with_capacity(4));
        assert_eq!(reserved.capacity(), 4);
        assert!(!reserved.shares_storage(&empty));
        let nodes = Shared::from_nodes(vec![changed]);
        let mut copy = nodes.clone();
        assert!(copy.shares_storage(&nodes));
        copy[0].children_mut().unwrap().clear();
        assert_eq!(nodes[0].children().len(), 1);
        assert!(copy[0].children().is_empty());
    }

    #[test]
    fn empty_children_cross_thread_clones_keep_identity_and_mutation_isolation() {
        use crate::shared::Shared;
        let original = Item::new(Ctor::Node, Default::default());
        let empty = original.as_item().unwrap().children.clone();
        let changed = std::thread::spawn(move || {
            let mut copy = original.clone();
            assert!(copy.as_item().unwrap().children.shares_storage(&empty));
            let local = Shared::empty_nodes();
            assert!(
                Item::new(Ctor::Node, Default::default())
                    .as_item()
                    .unwrap()
                    .children
                    .shares_storage(&local)
            );
            copy.children_mut()
                .unwrap()
                .push(Item::new(Ctor::Divider, Default::default()));
            assert!(original.children().is_empty());
            (copy, local)
        })
        .join()
        .unwrap();
        // The thread-local owner has dropped; escaped values remain valid.
        assert_eq!(changed.0.children().len(), 1);
        assert!(changed.1.is_empty());
        assert!(changed.1.clone().shares_storage(&changed.1));
    }

    #[test]
    fn empty_children_construction_during_thread_local_teardown_is_valid() {
        struct OnDrop;
        impl Drop for OnDrop {
            fn drop(&mut self) {
                let node = Item::new(Ctor::Node, Default::default());
                assert!(node.children().is_empty());
            }
        }
        thread_local! {
            static EARLIER: OnDrop = const { OnDrop };
        }
        std::thread::spawn(|| {
            EARLIER.with(|_| ());
            // Initialized later, hence dropped before EARLIER.
            drop(crate::shared::Shared::empty_nodes());
        })
        .join()
        .unwrap();
    }

    #[cfg(feature = "serde")]
    #[test]
    fn empty_children_wire_roundtrip_reuses_storage_and_keeps_origins() {
        let owner = crate::source::Source::metadata("empty.not").unwrap();
        let sources = crate::source::Sources::from(owner.clone());
        let mut original = Item::new(Ctor::Node, Default::default());
        original.origin = Some(Origin {
            source: owner.id(),
            start: 0,
            end: 0,
            expansion: Default::default(),
        });
        let wire = serde_json::to_value(sources.wire(&original)).unwrap();
        assert_eq!(wire["content"]["Item"]["children"], serde_json::json!([]));
        assert_eq!(wire["content"]["Item"]["fields"], serde_json::json!([]));
        assert_eq!(wire["attrs"], serde_json::json!([]));
        let decoded: Item = sources.decode(wire.clone()).unwrap();
        assert_eq!(decoded, original);
        assert!(
            decoded
                .as_item()
                .unwrap()
                .children
                .shares_storage(&original.as_item().unwrap().children)
        );
        assert_eq!(serde_json::to_value(sources.wire(&decoded)).unwrap(), wire);
    }

    #[test]
    fn item_instance_sharing_preserves_independent_identity_and_mounts() {
        use crate::definitions::{ComptimeValue, FunctionInstance, Type};
        let mut original = Item::new(Ctor::Node, TextRange::empty(0.into()));
        let mut definitions = crate::definition_identity::Definitions::default();
        original.as_item_mut().unwrap().instance = Some(
            definitions
                .resolve(FunctionInstance {
                    function: FunctionId::new("module", "view"),
                    arguments: vec![ComptimeValue::Type(Type::InlineContent)],
                })
                .into(),
        );
        let span = original.span;
        original
            .as_item_mut()
            .unwrap()
            .children
            .push(Item::text("kept".into(), span));
        let mut changed = original.clone();
        assert!(
            original
                .as_item()
                .unwrap()
                .instance
                .as_ref()
                .unwrap()
                .shares_storage(changed.as_item().unwrap().instance.as_ref().unwrap())
        );
        let data = changed.as_item_mut().unwrap();
        data.instance.as_mut().unwrap().function =
            definitions.insert(FunctionId::new("module", "different"));
        data.instance.as_mut().unwrap().arguments.clear();
        data.children.clear();
        assert_eq!(
            definitions
                .function(original.instance().unwrap().function)
                .unwrap()
                .name,
            "view"
        );
        assert_eq!(original.instance().unwrap().arguments.len(), 1);
        assert_eq!(original.children().len(), 1);
        assert_ne!(original, changed);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn item_instance_wire_remains_the_complete_value() {
        use crate::definitions::{ComptimeValue, FunctionInstance, Type};
        let mut item = Item::new(Ctor::Node, TextRange::empty(0.into()));
        let identity = FunctionInstance {
            function: FunctionId::new("module", "view"),
            arguments: vec![ComptimeValue::Type(Type::InlineContent)],
        };
        item.as_item_mut().unwrap().instance = Some(identity.clone().into());
        let mut sources = crate::source::Sources::new();
        sources.definitions_mut().insert(identity.function.clone());
        let wire = serde_json::to_value(sources.wire(&item)).unwrap();
        assert_eq!(
            wire["content"]["Item"]["instance"],
            serde_json::to_value(&identity).unwrap()
        );
        assert!(wire.get("pending").is_none());
        assert!(wire.get("evaluation").is_none());
        let restored: Item = sources.decode(wire).unwrap();
        assert_eq!(restored, item);
        assert_eq!(
            format!(
                "{:?}",
                sources
                    .definitions()
                    .selector(item.instance().unwrap())
                    .unwrap()
            ),
            format!("{identity:?}")
        );
    }

    #[test]
    fn origin_clones_share_identity_but_not_expansion_mutations() {
        let owner = crate::source::Source::metadata("modules/view.notc").unwrap();
        let another = crate::source::Source::metadata("another.notc").unwrap();
        let caller = crate::source::Source::metadata("caller.not").unwrap();
        let source = owner.id();
        let original = Origin {
            source,
            start: 1,
            end: 7,
            expansion: vec![SourceLocation {
                source,
                start: 10,
                end: 20,
            }]
            .into(),
        };
        let mut changed = original.clone();
        assert_eq!(original.source, changed.source);
        assert_eq!(original.expansion[0].source, changed.expansion[0].source);
        changed.source = another.id();
        changed.expansion[0].source = caller.id();
        changed.expansion.push(SourceLocation {
            source,
            start: 30,
            end: 40,
        });
        assert_eq!(original.source, owner.id());
        assert_eq!(original.expansion[0].source, owner.id());
        assert_eq!(original.expansion.len(), 1);
        assert_eq!(changed.expansion.len(), 2);
    }

    #[test]
    fn source_location_identity_remains_path_value_and_complete_span() {
        let owner = crate::source::Source::metadata("view.notc").unwrap();
        let different = crate::source::Source::metadata("different.notc").unwrap();
        let first = SourceLocation {
            source: owner.id(),
            start: 2,
            end: 8,
        };
        let mut equal = SourceLocation {
            source: owner.id(),
            start: 2,
            end: 8,
        };
        assert_eq!(first, equal);
        equal.end += 1;
        assert_ne!(first, equal);
        equal.end -= 1;
        equal.source = different.id();
        assert_ne!(first, equal);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn source_wire_remains_plain_path_values() {
        let source = "dir/\"文档\".notc";
        let owner = crate::source::Source::metadata(source).unwrap();
        let caller = crate::source::Source::metadata("caller.not").unwrap();
        let mut sources = crate::source::Sources::from(owner.clone());
        sources.insert(caller.clone()).unwrap();
        let mut origin = Origin {
            source: owner.id(),
            start: 1,
            end: 7,
            expansion: vec![SourceLocation {
                source: caller.id(),
                start: 10,
                end: 20,
            }]
            .into(),
        };
        let expected = serde_json::json!({
            "source": source, "start": 1, "end": 7,
            "expansion": [{"source": "caller.not", "start": 10, "end": 20}],
        });
        assert_eq!(
            serde_json::to_value(sources.wire(&origin)).unwrap(),
            expected
        );
        let restored: Origin = sources.decode(expected).unwrap();
        assert_eq!(restored, origin);
        origin.expansion.clear();
        assert_eq!(
            format!("{origin:?}"),
            format!(
                "Origin {{ source: {:?}, start: 1, end: 7, expansion: [] }}",
                owner.id()
            )
        );
    }
}
