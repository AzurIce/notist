use rowan::TextRange;

use crate::builtins;
use crate::definitions::FunctionId;

/// A registered call's public contract snapshot, independent of its module files.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ExtensionCtor {
    pub id: FunctionId,
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

    pub fn name(&self) -> std::borrow::Cow<'_, str> {
        if let Self::Extension(function) = self {
            return function.id.to_string().into();
        }
        std::borrow::Cow::Borrowed(match self {
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
        })
    }

    /// Canonical identity for both builtin and registered external calls.
    pub fn function_id(&self) -> Option<FunctionId> {
        match self {
            Self::Extension(function) => Some(function.id.clone()),
            _ => self
                .source_name()
                .map(|name| FunctionId::new("notist", name)),
        }
    }
}

impl std::fmt::Display for Ctor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name())
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
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Origin {
    /// Interned source name, independent of the owning result's source revision.
    pub source: crate::source::SourceId,
    pub start: u32,
    pub end: u32,
    pub expansion: crate::source_chain::SourceChain,
}
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SourceLocation {
    pub source: crate::source::SourceId,
    pub start: u32,
    pub end: u32,
}
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Node {
    pub content: NodeContent,
    pub attrs: Dict,
    #[cfg_attr(feature = "serde", serde(skip_serializing_if = "Option::is_none"))]
    pub origin: Option<Origin>,
    /// Resolved content category. Document analysis always stores Block or Inline.
    pub level: builtins::Level,
    #[cfg_attr(feature = "serde", serde(with = "crate::evaluation::span"))]
    pub span: TextRange,
}

/// A content tree can contain expressions before evaluation. Complete output
/// contains only `Item` variants; transparent groups are ordinary content.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum NodeContent {
    Item(ContentItem),
    Expr(crate::shared::Shared<crate::evaluation::TypedExpr>),
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ContentItem {
    pub ctor: Ctor,
    pub fields: Dict,
    pub children: crate::shared::Shared<Vec<Node>>,
    /// Concrete native/component parameters, not a source call or execution state.
    pub instance: Option<crate::shared::Shared<crate::definitions::FunctionInstance>>,
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
                let def = &call.definition;
                if def.id.module == "notist" {
                    Ctor::from_name(&def.id.name).unwrap_or(Ctor::Interpolation)
                } else {
                    Ctor::Extension(ExtensionCtor {
                        id: def.id.clone(),
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
        if name == "level" && call.instance.function == FunctionId::new("notist", "heading") {
            if let Some(crate::definitions::ComptimeValue::Data(value)) =
                call.instance.arguments.first()
            {
                return Some(value);
            }
        }
        let (_, arg) = call.arguments.iter().find(|(key, _)| key == name)?;
        match arg.known.as_ref() {
            Some(crate::definitions::ComptimeValue::Data(value)) => {
                if call.definition.parameters.iter().any(|p| {
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
        for (name, _) in &call.arguments {
            if let Some(value) = self.field(name) {
                fields.insert(name.clone(), value.clone());
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
    pub fn instance(&self) -> Option<&crate::definitions::FunctionInstance> {
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
            source: expr.source.as_str().into(),
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
    pub fn function_id(&self) -> Option<FunctionId> {
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
                children: Vec::new().into(),
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

    pub fn with_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.as_item_mut()
            .expect("content constructor")
            .fields
            .insert(key, value);
        self.infer_level();
        self
    }

    pub fn with_children(mut self, children: Vec<Item>) -> Self {
        self.as_item_mut().expect("content constructor").children = children.into();
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

    /// Depth-first iteration over this item and all its descendants.
    pub fn descendants(&self) -> impl Iterator<Item = &Item> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let item = stack.pop()?;
            stack.extend(item.children().iter().rev());
            Some(item)
        })
    }

    /// The first descendant (self included) matching `pred`.
    pub fn find(&self, pred: impl Fn(&Item) -> bool) -> Option<&Item> {
        self.descendants().find(|item| pred(item))
    }
}

#[cfg(test)]
mod source_tests {
    use super::*;

    #[test]
    fn item_instance_sharing_preserves_independent_identity_and_mounts() {
        use crate::definitions::{ComptimeValue, FunctionInstance, Type};
        let mut original = Item::new(Ctor::Node, TextRange::empty(0.into()));
        original.as_item_mut().unwrap().instance = Some(
            FunctionInstance {
                function: FunctionId::new("module", "view"),
                arguments: vec![ComptimeValue::Type(Type::InlineContent)],
            }
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
        data.instance.as_mut().unwrap().function.name = "different".into();
        data.instance.as_mut().unwrap().arguments.clear();
        data.children.clear();
        assert_eq!(original.instance().unwrap().function.name, "view");
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
        let wire = serde_json::to_value(&item).unwrap();
        assert_eq!(
            wire["content"]["Item"]["instance"],
            serde_json::to_value(&identity).unwrap()
        );
        assert!(wire.get("pending").is_none());
        assert!(wire.get("evaluation").is_none());
        let restored: Item = serde_json::from_value(wire).unwrap();
        assert_eq!(restored, item);
        assert_eq!(
            format!("{:?}", item.instance().unwrap()),
            format!("{identity:?}")
        );
    }

    #[test]
    fn origin_clones_copy_source_identity_but_not_expansion_mutations() {
        let source: crate::source::SourceId = "modules/view.notc".into();
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
        changed.source = "another.notc".into();
        changed.expansion[0].source = "caller.not".into();
        changed.expansion.push(SourceLocation {
            source,
            start: 30,
            end: 40,
        });
        assert_eq!(original.source, "modules/view.notc");
        assert_eq!(original.expansion[0].source, "modules/view.notc");
        assert_eq!(original.expansion.len(), 1);
        assert_eq!(changed.expansion.len(), 2);
    }

    #[test]
    fn source_location_identity_remains_path_value_and_complete_span() {
        let first = SourceLocation {
            source: "view.notc".into(),
            start: 2,
            end: 8,
        };
        let mut equal = SourceLocation {
            source: "view.notc".into(),
            start: 2,
            end: 8,
        };
        assert_eq!(first.source, equal.source);
        assert_eq!(first, equal);
        equal.end += 1;
        assert_ne!(first, equal);
        equal.end -= 1;
        equal.source = "different.notc".into();
        assert_ne!(first, equal);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn source_wire_and_debug_remain_plain_path_values() {
        let source = "dir/\"文档\".notc";
        let mut origin = Origin {
            source: source.into(),
            start: 1,
            end: 7,
            expansion: vec![SourceLocation {
                source: "caller.not".into(),
                start: 10,
                end: 20,
            }]
            .into(),
        };
        let expected = serde_json::json!({
            "source": source, "start": 1, "end": 7,
            "expansion": [{"source": "caller.not", "start": 10, "end": 20}],
        });
        assert_eq!(serde_json::to_value(&origin).unwrap(), expected);
        let restored: Origin = serde_json::from_value(expected).unwrap();
        assert_eq!(restored, origin);
        origin.expansion.clear();
        assert_eq!(
            format!("{origin:?}"),
            format!("Origin {{ source: {source:?}, start: 1, end: 7, expansion: [] }}")
        );
    }
}
