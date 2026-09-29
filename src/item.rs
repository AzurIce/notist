use rowan::TextRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ctor {
    Doc,
    Paragraph,
    Heading,
    Text,
    RawInline,
    Strong,
    Emph,
    Math,
    Link,
    CodeEmbed,
    List,
    ListItem,
}

impl Ctor {
    pub fn name(self) -> &'static str {
        match self {
            Ctor::Doc => "Doc",
            Ctor::Paragraph => "Paragraph",
            Ctor::Heading => "Heading",
            Ctor::Text => "Text",
            Ctor::RawInline => "RawInline",
            Ctor::Strong => "Strong",
            Ctor::Emph => "Emph",
            Ctor::Math => "Math",
            Ctor::Link => "Link",
            Ctor::CodeEmbed => "CodeEmbed",
            Ctor::List => "List",
            Ctor::ListItem => "ListItem",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dict(Vec<(String, Value)>);

impl Dict {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
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
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Unit => write!(f, "()"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Str(s) => write!(f, "{s:?}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub ctor: Ctor,
    pub fields: Dict,
    pub children: Vec<Item>,
    pub span: TextRange,
}

impl Item {
    pub fn new(ctor: Ctor, span: TextRange) -> Self {
        Self {
            ctor,
            fields: Dict::default(),
            children: Vec::new(),
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
}
