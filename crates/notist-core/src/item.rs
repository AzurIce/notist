use rowan::TextRange;

use crate::builtins;

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
    /// and structure without adding semantics. Produced by `#[..]`.
    Group,
    /// A document section: produced by `sectionize`, the identity carrier of
    /// its range — attrs are transferred from the heading that opens it (a
    /// heading opens exactly one section), and the span covers the whole
    /// extent. The heading stays the first child. The source spelling
    /// `#section[..]` resolves to the same constructor.
    Section,
    /// Recovery representation of a call to an unknown name — always
    /// accompanied by an `unknown constructor` diagnostic, never a
    /// legitimate constructor.
    Custom(String),
}

impl Ctor {
    /// The source-level name of a builtin constructor (`Doc` and custom
    /// constructors have none).
    fn source_name(&self) -> Option<&'static str> {
        Some(match self {
            Ctor::Doc | Ctor::Custom(_) => return None,
            Ctor::Paragraph => "paragraph",
            Ctor::Heading => "heading",
            Ctor::Text => "text",
            Ctor::RawInline => "raw",
            Ctor::Strong => "strong",
            Ctor::Emph => "emph",
            Ctor::Strike => "strike",
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
            Ctor::Group => "group",
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
            "group" => Ctor::Group,
            "section" => Ctor::Section,
            _ => return None,
        })
    }

    /// The element level of a builtin constructor, if statically known.
    /// `Doc` and custom constructors return `None` (derive it structurally).
    pub fn level(&self) -> Option<builtins::Level> {
        self.source_name()
            .and_then(builtins::builtin_signature)
            .map(|s| s.level)
    }

    /// What a builtin constructor's children mount accepts.
    pub fn accepts(&self) -> Option<builtins::Accepts> {
        self.source_name()
            .and_then(builtins::builtin_signature)
            .map(|s| s.accepts)
    }

    pub fn name(&self) -> &str {
        match self {
            Ctor::Doc => "Doc",
            Ctor::Paragraph => "Paragraph",
            Ctor::Heading => "Heading",
            Ctor::Text => "Text",
            Ctor::RawInline => "RawInline",
            Ctor::Strong => "Strong",
            Ctor::Emph => "Emph",
            Ctor::Strike => "Strike",
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
            Ctor::Group => "Group",
            Ctor::Section => "Section",
            Ctor::Custom(name) => name,
        }
    }
}

impl std::fmt::Display for Ctor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Array(Vec<Value>),
    Dict(Dict),
}

#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Dict(Vec<(String, Value)>);

impl Dict {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn insert(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        match self.0.iter_mut().find(|(k, _)| *k == key) {
            Some((_, v)) => *v = value,
            None => self.0.push((key, value)),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.0.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn extend(&mut self, other: Dict) {
        for (k, v) in other.0 {
            self.insert(k, v);
        }
    }

    pub fn take(&mut self) -> Dict {
        std::mem::take(self)
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
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Item {
    pub ctor: Ctor,
    pub fields: Dict,
    pub children: Vec<Item>,
    pub attrs: Dict,
    #[cfg_attr(feature = "serde", serde(skip, default = "empty_span"))]
    pub span: TextRange,
}

#[cfg(feature = "serde")]
fn empty_span() -> TextRange {
    TextRange::empty(0.into())
}

impl Item {
    pub fn new(ctor: Ctor, span: TextRange) -> Self {
        Self {
            ctor,
            fields: Dict::default(),
            children: Vec::new(),
            attrs: Dict::default(),
            span,
        }
    }

    pub fn text(text: String, span: TextRange) -> Self {
        Item::new(Ctor::Text, span).with_field("text", Value::Str(text))
    }

    pub fn with_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.fields.insert(key, value);
        self
    }

    pub fn with_children(mut self, children: Vec<Item>) -> Self {
        self.children = children;
        self
    }

    /// Depth-first iteration over this item and all its descendants.
    pub fn descendants(&self) -> impl Iterator<Item = &Item> {
        let mut stack = vec![self];
        std::iter::from_fn(move || {
            let item = stack.pop()?;
            stack.extend(item.children.iter().rev());
            Some(item)
        })
    }

    /// The first descendant (self included) matching `pred`.
    pub fn find(&self, pred: impl Fn(&Item) -> bool) -> Option<&Item> {
        self.descendants().find(|item| pred(item))
    }
}
