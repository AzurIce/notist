//! Explicit, borrowing wire projections. Runtime IDs never implement serde.
//! Import DTOs are temporary boundary data, not a second runtime content tree.
use crate::{
    builtins::Level,
    definitions::{ComptimeValue, FunctionDef, FunctionInstance, Type},
    evaluation::{
        CallSite, ContentShape, ExprKind, Invocation, ResultArm, ResultPattern, TypedExpr,
    },
    item::{ContentItem, Ctor, Node, NodeContent, Origin, SourceLocation},
    shared::{Dict, Shared},
    source::{Located, Source, SourceError, SourceId, SourceNamespace, Sources},
    source_chain::SourceChain,
};
use rowan::TextRange;
use serde::{
    Deserialize, Serialize, Serializer,
    ser::{Error as _, SerializeSeq},
};

pub struct Wire<'a, T: ?Sized> {
    value: &'a T,
    sources: &'a Sources,
}
pub trait SerializeWithSources {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error>;
}
impl<T: SerializeWithSources + ?Sized> Serialize for Wire<'_, T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize_with(self.sources, serializer)
    }
}
impl Sources {
    pub fn wire<'a, T: SerializeWithSources + ?Sized>(&'a self, value: &'a T) -> Wire<'a, T> {
        Wire {
            value,
            sources: self,
        }
    }
    /// Decode an extension response against this exact input view. No unknown
    /// source can be silently added to an immutable document context.
    pub fn decode<T: DeserializeWithSources>(
        &self,
        value: serde_json::Value,
    ) -> Result<T, serde_json::Error> {
        let wire = serde_json::from_value::<T::Input>(value)?;
        T::decode(
            wire,
            &mut Input {
                sources: self.clone(),
                namespace: SourceNamespace::default(),
                allow_unknown: false,
            },
        )
        .map_err(serde::de::Error::custom)
    }
    /// Import legacy wire names with explicit domain and retained ownership.
    /// Wire without source text creates metadata-only records, never fake text.
    pub fn import<T: DeserializeWithSources>(
        value: serde_json::Value,
        namespace: SourceNamespace,
    ) -> Result<Located<T>, serde_json::Error> {
        let wire = serde_json::from_value::<T::Input>(value)?;
        let mut input = Input {
            sources: Sources::new(),
            namespace,
            allow_unknown: true,
        };
        let value = T::decode(wire, &mut input).map_err(serde::de::Error::custom)?;
        Ok(Located::new(value, input.sources))
    }
}
impl<T: SerializeWithSources + ?Sized> SerializeWithSources for &T {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        (*self).serialize_with(sources, serializer)
    }
}
impl<T: SerializeWithSources> SerializeWithSources for Option<T> {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match self {
            Some(value) => serializer.serialize_some(&sources.wire(value)),
            None => serializer.serialize_none(),
        }
    }
}
impl<T: SerializeWithSources> SerializeWithSources for [T] {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.len()))?;
        for value in self {
            seq.serialize_element(&sources.wire(value))?;
        }
        seq.end()
    }
}
impl<T: SerializeWithSources> SerializeWithSources for Vec<T> {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        self.as_slice().serialize_with(sources, serializer)
    }
}
impl SerializeWithSources for SourceId {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let name = sources.name(*self).map_err(S::Error::custom)?;
        if *self != SourceId::DETACHED
            && sources.by_name(name).map_err(S::Error::custom)?.id() != *self
        {
            return Err(S::Error::custom(
                "source name does not identify this source",
            ));
        }
        serializer.serialize_str(name)
    }
}

struct Arguments<'a>(&'a [(String, TypedExpr)], &'a Sources);
impl Serialize for Arguments<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for (name, value) in self.0 {
            seq.serialize_element(&(name, self.1.wire(value)))?;
        }
        seq.end()
    }
}

#[derive(Serialize)]
struct NodeOut<'a> {
    content: Wire<'a, NodeContent>,
    attrs: &'a Dict,
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<Wire<'a, Origin>>,
    level: Level,
    span: (u32, u32),
}
fn range(span: TextRange) -> (u32, u32) {
    (span.start().into(), span.end().into())
}
impl SerializeWithSources for Node {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        NodeOut {
            content: sources.wire(&self.content),
            attrs: &self.attrs,
            origin: self.origin.as_ref().map(|v| sources.wire(v)),
            level: self.level,
            span: range(self.span),
        }
        .serialize(serializer)
    }
}
#[derive(Serialize)]
enum NodeContentOut<'a> {
    Item(Wire<'a, ContentItem>),
    Expr(Wire<'a, TypedExpr>),
}
impl SerializeWithSources for NodeContent {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match self {
            Self::Item(v) => NodeContentOut::Item(sources.wire(v)),
            Self::Expr(v) => NodeContentOut::Expr(sources.wire(&**v)),
        }
        .serialize(serializer)
    }
}
#[derive(Serialize)]
struct ContentItemOut<'a> {
    ctor: &'a Ctor,
    fields: &'a Dict,
    children: Wire<'a, [Node]>,
    instance: &'a Option<Shared<FunctionInstance>>,
}
impl SerializeWithSources for ContentItem {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        ContentItemOut {
            ctor: &self.ctor,
            fields: &self.fields,
            children: sources.wire(self.children.as_slice()),
            instance: &self.instance,
        }
        .serialize(serializer)
    }
}
#[derive(Serialize)]
struct OriginOut<'a> {
    source: Wire<'a, SourceId>,
    start: u32,
    end: u32,
    expansion: Wire<'a, [SourceLocation]>,
}
impl SerializeWithSources for Origin {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        OriginOut {
            source: sources.wire(&self.source),
            start: self.start,
            end: self.end,
            expansion: sources.wire(self.expansion.as_slice()),
        }
        .serialize(serializer)
    }
}
#[derive(Serialize)]
struct LocationOut<'a> {
    source: Wire<'a, SourceId>,
    start: u32,
    end: u32,
}
impl SerializeWithSources for SourceLocation {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        LocationOut {
            source: sources.wire(&self.source),
            start: self.start,
            end: self.end,
        }
        .serialize(serializer)
    }
}
impl SerializeWithSources for SourceChain {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        self.as_slice().serialize_with(sources, serializer)
    }
}
#[derive(Serialize)]
struct ExpressionOut<'a> {
    ty: &'a Option<Type>,
    known: &'a Option<ComptimeValue>,
    kind: Wire<'a, ExprKind>,
    span: (u32, u32),
    source: Wire<'a, SourceId>,
    shape: ContentShape,
}
impl SerializeWithSources for TypedExpr {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        ExpressionOut {
            ty: &self.ty,
            known: &self.known,
            kind: sources.wire(&self.kind),
            span: range(self.span),
            source: sources.wire(&self.source),
            shape: self.shape,
        }
        .serialize(serializer)
    }
}
#[derive(Serialize)]
enum KindOut<'a> {
    Constant(&'a ComptimeValue),
    Input(&'a str),
    Array(Wire<'a, [TypedExpr]>),
    Dict(Arguments<'a>),
    Unary(&'a str, Wire<'a, TypedExpr>),
    Binary(&'a str, Wire<'a, TypedExpr>, Wire<'a, TypedExpr>),
    Block(Arguments<'a>, Wire<'a, TypedExpr>),
    If(
        Wire<'a, TypedExpr>,
        Wire<'a, TypedExpr>,
        Wire<'a, TypedExpr>,
    ),
    Some(Wire<'a, TypedExpr>),
    Unwrap(Wire<'a, TypedExpr>),
    Ok(Wire<'a, TypedExpr>),
    Err(Wire<'a, TypedExpr>),
    MatchResult {
        subject: Wire<'a, TypedExpr>,
        arms: Wire<'a, [ResultArm]>,
    },
    Call(Wire<'a, Invocation>),
    Content(Wire<'a, [Node]>),
    For {
        binding: &'a str,
        subject: Wire<'a, TypedExpr>,
        body: Wire<'a, TypedExpr>,
    },
    Failure(&'a str),
}
impl SerializeWithSources for ExprKind {
    fn serialize_with<S: Serializer>(&self, s: &Sources, serializer: S) -> Result<S::Ok, S::Error> {
        let out = match self {
            Self::Constant(v) => KindOut::Constant(v),
            Self::Input(v) => KindOut::Input(v),
            Self::Array(v) => KindOut::Array(s.wire(v.as_slice())),
            Self::Dict(v) => KindOut::Dict(Arguments(v, s)),
            Self::Unary(op, v) => KindOut::Unary(op, s.wire(&**v)),
            Self::Binary(op, a, b) => KindOut::Binary(op, s.wire(&**a), s.wire(&**b)),
            Self::Block(bindings, value) => {
                KindOut::Block(Arguments(bindings, s), s.wire(&**value))
            }
            Self::If(a, b, c) => KindOut::If(s.wire(&**a), s.wire(&**b), s.wire(&**c)),
            Self::Some(v) => KindOut::Some(s.wire(&**v)),
            Self::Unwrap(v) => KindOut::Unwrap(s.wire(&**v)),
            Self::Ok(v) => KindOut::Ok(s.wire(&**v)),
            Self::Err(v) => KindOut::Err(s.wire(&**v)),
            Self::MatchResult { subject, arms } => KindOut::MatchResult {
                subject: s.wire(&**subject),
                arms: s.wire(arms.as_slice()),
            },
            Self::Call(v) => KindOut::Call(s.wire(&**v)),
            Self::Content(v) => KindOut::Content(s.wire(v.as_slice())),
            Self::For {
                binding,
                subject,
                body,
            } => KindOut::For {
                binding,
                subject: s.wire(&**subject),
                body: s.wire(&**body),
            },
            Self::Failure(v) => KindOut::Failure(v),
        };
        out.serialize(serializer)
    }
}
#[derive(Serialize)]
struct ArmOut<'a> {
    pattern: &'a ResultPattern,
    body: Wire<'a, TypedExpr>,
}
impl SerializeWithSources for ResultArm {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        ArmOut {
            pattern: &self.pattern,
            body: sources.wire(&self.body),
        }
        .serialize(serializer)
    }
}
#[derive(Serialize)]
struct InvocationOut<'a> {
    instance: &'a FunctionInstance,
    definition: &'a FunctionDef,
    arguments: Arguments<'a>,
    children: Option<Wire<'a, TypedExpr>>,
    attrs: &'a Dict,
    span: (u32, u32),
    source: Wire<'a, SourceId>,
    #[serde(skip_serializing_if = "CallSite::is_markup")]
    site: CallSite,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource_root: &'a Option<String>,
    invalid: &'a Option<String>,
}
impl SerializeWithSources for Invocation {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        InvocationOut {
            instance: &self.instance,
            definition: &self.definition,
            arguments: Arguments(&self.arguments, sources),
            children: self.children.as_deref().map(|v| sources.wire(v)),
            attrs: &self.attrs,
            span: range(self.span),
            source: sources.wire(&self.source),
            site: self.site,
            resource_root: &self.resource_root,
            invalid: &self.invalid,
        }
        .serialize(serializer)
    }
}

pub struct Input {
    sources: Sources,
    namespace: SourceNamespace,
    allow_unknown: bool,
}
impl Input {
    fn source(&mut self, name: &str) -> Result<SourceId, SourceError> {
        if name.is_empty() {
            return Ok(SourceId::DETACHED);
        }
        match self.sources.by_name(name) {
            Ok(source) => Ok(source.id()),
            Err(SourceError::Unknown { .. }) if self.allow_unknown => self
                .sources
                .insert(Source::in_namespace(self.namespace.clone(), name, None)?),
            Err(error) => Err(error),
        }
    }
}
pub trait DeserializeWithSources: Sized {
    type Input: serde::de::DeserializeOwned;
    fn decode(input: Self::Input, sources: &mut Input) -> Result<Self, SourceError>;
}
#[derive(Deserialize)]
pub struct NodeIn {
    content: ContentIn,
    attrs: Dict,
    origin: Option<OriginIn>,
    level: Level,
    #[serde(with = "crate::evaluation::span")]
    span: TextRange,
}
#[derive(Deserialize)]
enum ContentIn {
    Item(ItemIn),
    Expr(Box<ExpressionIn>),
}
#[derive(Deserialize)]
struct ItemIn {
    ctor: Ctor,
    fields: Dict,
    children: Vec<NodeIn>,
    instance: Option<Shared<FunctionInstance>>,
}
#[derive(Deserialize)]
pub struct OriginIn {
    source: String,
    start: u32,
    end: u32,
    expansion: Vec<LocationIn>,
}
#[derive(Deserialize)]
pub struct LocationIn {
    source: String,
    start: u32,
    end: u32,
}
#[derive(Deserialize)]
pub struct ExpressionIn {
    ty: Option<Type>,
    known: Option<ComptimeValue>,
    kind: KindIn,
    #[serde(with = "crate::evaluation::span")]
    span: TextRange,
    source: String,
    #[serde(default)]
    shape: ContentShape,
}
#[derive(Deserialize)]
enum KindIn {
    Constant(ComptimeValue),
    Input(String),
    Array(Vec<ExpressionIn>),
    Dict(Vec<(String, ExpressionIn)>),
    Unary(String, Box<ExpressionIn>),
    Binary(String, Box<ExpressionIn>, Box<ExpressionIn>),
    Block(Vec<(String, ExpressionIn)>, Box<ExpressionIn>),
    If(Box<ExpressionIn>, Box<ExpressionIn>, Box<ExpressionIn>),
    Some(Box<ExpressionIn>),
    Unwrap(Box<ExpressionIn>),
    Ok(Box<ExpressionIn>),
    Err(Box<ExpressionIn>),
    MatchResult {
        subject: Box<ExpressionIn>,
        arms: Vec<ArmIn>,
    },
    Call(InvocationIn),
    Content(Vec<NodeIn>),
    For {
        binding: String,
        subject: Box<ExpressionIn>,
        body: Box<ExpressionIn>,
    },
    Failure(String),
}
#[derive(Deserialize)]
struct ArmIn {
    pattern: ResultPattern,
    body: ExpressionIn,
}
#[derive(Deserialize)]
pub struct InvocationIn {
    instance: FunctionInstance,
    definition: Shared<FunctionDef>,
    arguments: Vec<(String, ExpressionIn)>,
    children: Option<Box<ExpressionIn>>,
    attrs: Dict,
    #[serde(with = "crate::evaluation::span")]
    span: TextRange,
    source: String,
    #[serde(default)]
    site: CallSite,
    #[serde(default)]
    resource_root: Option<String>,
    invalid: Option<String>,
}
fn expressions(values: Vec<ExpressionIn>, s: &mut Input) -> Result<Vec<TypedExpr>, SourceError> {
    values
        .into_iter()
        .map(|v| TypedExpr::decode(v, s))
        .collect()
}
fn arguments(
    values: Vec<(String, ExpressionIn)>,
    s: &mut Input,
) -> Result<Vec<(String, TypedExpr)>, SourceError> {
    values
        .into_iter()
        .map(|(name, v)| Ok((name, TypedExpr::decode(v, s)?)))
        .collect()
}
fn expression(v: ExpressionIn, s: &mut Input) -> Result<Box<TypedExpr>, SourceError> {
    Ok(Box::new(TypedExpr::decode(v, s)?))
}
impl DeserializeWithSources for Node {
    type Input = NodeIn;
    fn decode(v: NodeIn, s: &mut Input) -> Result<Self, SourceError> {
        Ok(Self {
            content: match v.content {
                ContentIn::Item(i) => NodeContent::Item(ContentItem {
                    ctor: i.ctor,
                    fields: i.fields,
                    children: i
                        .children
                        .into_iter()
                        .map(|n| Node::decode(n, s))
                        .collect::<Result<Vec<_>, _>>()?
                        .into(),
                    instance: i.instance,
                }),
                ContentIn::Expr(e) => NodeContent::Expr(TypedExpr::decode(*e, s)?.into()),
            },
            attrs: v.attrs,
            origin: v.origin.map(|o| Origin::decode(o, s)).transpose()?,
            level: v.level,
            span: v.span,
        })
    }
}
impl DeserializeWithSources for SourceLocation {
    type Input = LocationIn;
    fn decode(v: LocationIn, s: &mut Input) -> Result<Self, SourceError> {
        if v.start > v.end {
            return Err(SourceError::Conflict {
                name: "reversed source range".into(),
            });
        }
        Ok(Self {
            source: s.source(&v.source)?,
            start: v.start,
            end: v.end,
        })
    }
}
impl DeserializeWithSources for Origin {
    type Input = OriginIn;
    fn decode(v: OriginIn, s: &mut Input) -> Result<Self, SourceError> {
        if v.start > v.end {
            return Err(SourceError::Conflict {
                name: "reversed source range".into(),
            });
        }
        Ok(Self {
            source: s.source(&v.source)?,
            start: v.start,
            end: v.end,
            expansion: v
                .expansion
                .into_iter()
                .map(|v| SourceLocation::decode(v, s))
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        })
    }
}
impl DeserializeWithSources for TypedExpr {
    type Input = ExpressionIn;
    fn decode(v: ExpressionIn, s: &mut Input) -> Result<Self, SourceError> {
        let kind = match v.kind {
            KindIn::Constant(v) => ExprKind::Constant(v),
            KindIn::Input(v) => ExprKind::Input(v),
            KindIn::Array(v) => ExprKind::Array(expressions(v, s)?),
            KindIn::Dict(v) => ExprKind::Dict(arguments(v, s)?),
            KindIn::Unary(op, v) => ExprKind::Unary(op, expression(*v, s)?),
            KindIn::Binary(op, a, b) => {
                ExprKind::Binary(op, expression(*a, s)?, expression(*b, s)?)
            }
            KindIn::Block(v, e) => ExprKind::Block(arguments(v, s)?, expression(*e, s)?),
            KindIn::If(a, b, c) => {
                ExprKind::If(expression(*a, s)?, expression(*b, s)?, expression(*c, s)?)
            }
            KindIn::Some(v) => ExprKind::Some(expression(*v, s)?),
            KindIn::Unwrap(v) => ExprKind::Unwrap(expression(*v, s)?),
            KindIn::Ok(v) => ExprKind::Ok(expression(*v, s)?),
            KindIn::Err(v) => ExprKind::Err(expression(*v, s)?),
            KindIn::MatchResult { subject, arms } => ExprKind::MatchResult {
                subject: expression(*subject, s)?,
                arms: arms
                    .into_iter()
                    .map(|v| {
                        Ok(ResultArm {
                            pattern: v.pattern,
                            body: TypedExpr::decode(v.body, s)?,
                        })
                    })
                    .collect::<Result<_, SourceError>>()?,
            },
            KindIn::Call(v) => ExprKind::Call(Invocation::decode(v, s)?.into()),
            KindIn::Content(v) => ExprKind::Content(
                v.into_iter()
                    .map(|v| Node::decode(v, s))
                    .collect::<Result<Vec<_>, _>>()?
                    .into(),
            ),
            KindIn::For {
                binding,
                subject,
                body,
            } => ExprKind::For {
                binding,
                subject: TypedExpr::decode(*subject, s)?.into(),
                body: TypedExpr::decode(*body, s)?.into(),
            },
            KindIn::Failure(v) => ExprKind::Failure(v),
        };
        Ok(Self {
            ty: v.ty,
            known: v.known,
            kind,
            span: v.span,
            source: s.source(&v.source)?,
            shape: v.shape,
        })
    }
}
impl DeserializeWithSources for Invocation {
    type Input = InvocationIn;
    fn decode(v: InvocationIn, s: &mut Input) -> Result<Self, SourceError> {
        Ok(Self {
            instance: v.instance,
            definition: v.definition,
            arguments: arguments(v.arguments, s)?,
            children: v.children.map(|v| expression(*v, s)).transpose()?,
            attrs: v.attrs,
            span: v.span,
            source: s.source(&v.source)?,
            site: v.site,
            resource_root: v.resource_root,
            invalid: v.invalid,
        })
    }
}
