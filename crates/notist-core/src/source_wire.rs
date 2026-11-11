//! Explicit, borrowing wire projections. Runtime IDs never implement serde.
//! Import DTOs are temporary boundary data, not a second runtime content tree.
use crate::{
    builtins::Level,
    definitions::{ComptimeValue, FunctionDef, ResolvedInstance, Type},
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
        self.decode_with_bindings(value, &Default::default())
    }
    /// Decode an extracted body with its explicit free lexical environment.
    /// Enclosing let/loop/match declarations still shadow this environment.
    pub fn decode_with_bindings<T: DeserializeWithSources>(
        &self,
        value: serde_json::Value,
        bindings: &std::collections::BTreeMap<String, crate::binding::BindingId>,
    ) -> Result<T, serde_json::Error> {
        for (name, id) in bindings {
            self.validate_binding(*id)
                .map_err(<serde_json::Error as serde::de::Error>::custom)?;
            let info = self
                .bindings()
                .get(*id)
                .map_err(<serde_json::Error as serde::de::Error>::custom)?;
            if &info.name != name {
                return Err(serde::de::Error::custom(
                    "lexical input name does not match its retained declaration",
                ));
            }
        }
        let wire = serde_json::from_value::<T::Input>(value)?;
        T::decode(
            wire,
            &mut Input {
                sources: self.clone(),
                namespace: SourceNamespace::default(),
                allow_unknown: false,
                locals: bindings.clone(),
            },
        )
        .map_err(serde::de::Error::custom)
    }
    /// Import explicit wire names and declaration addresses with retained ownership.
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
            locals: Default::default(),
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
impl SerializeWithSources for crate::binding::BindingId {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct BindingOut<'a> {
            name: &'a str,
            source: Wire<'a, SourceId>,
            span: (u32, u32),
            scope: (u32, u32),
            #[serde(skip_serializing_if = "Option::is_none")]
            owner: Option<&'a crate::definitions::FunctionId>,
        }
        let info = sources.bindings().get(*self).map_err(S::Error::custom)?;
        BindingOut {
            owner: info.owner.as_ref(),
            name: &info.name,
            source: sources.wire(&info.source),
            span: range(info.span),
            scope: range(info.scope),
        }
        .serialize(serializer)
    }
}
impl SerializeWithSources for ResolvedInstance {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Out<'a> {
            function: &'a crate::definitions::FunctionId,
            arguments: &'a [ComptimeValue],
        }
        Out {
            function: sources
                .definitions()
                .function(self.function)
                .map_err(S::Error::custom)?,
            arguments: &self.arguments,
        }
        .serialize(serializer)
    }
}
impl SerializeWithSources for Ctor {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        if let Ctor::Extension(extension) = self {
            #[derive(Serialize)]
            enum Out<'a> {
                Extension {
                    id: &'a crate::definitions::FunctionId,
                    accepts: crate::builtins::Accepts,
                    level: Level,
                },
            }
            Out::Extension {
                id: sources
                    .definitions()
                    .function(extension.id)
                    .map_err(S::Error::custom)?,
                accepts: extension.accepts,
                level: extension.level,
            }
            .serialize(serializer)
        } else {
            self.serialize(serializer)
        }
    }
}

struct Arguments<'a>(&'a [(String, TypedExpr)], &'a Sources);
struct CallArguments<'a>(&'a Invocation, &'a Sources);
impl Serialize for CallArguments<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.checked_arguments().len()))?;
        for (name, value) in self.0.arguments() {
            seq.serialize_element(&(name, self.1.wire(value)))?;
        }
        seq.end()
    }
}
struct LexicalArguments<'a>(&'a [(crate::binding::BindingId, TypedExpr)], &'a Sources);
impl Serialize for LexicalArguments<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for (id, value) in self.0 {
            seq.serialize_element(&(self.1.wire(id), self.1.wire(value)))?;
        }
        seq.end()
    }
}
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
    ctor: Wire<'a, Ctor>,
    fields: &'a Dict,
    children: Wire<'a, [Node]>,
    instance: Option<Wire<'a, ResolvedInstance>>,
}
impl SerializeWithSources for ContentItem {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        ContentItemOut {
            ctor: sources.wire(&self.ctor),
            fields: &self.fields,
            children: sources.wire(self.children.as_slice()),
            instance: self.instance.as_deref().map(|value| sources.wire(value)),
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
    Input(Wire<'a, crate::binding::BindingId>),
    Array(Wire<'a, [TypedExpr]>),
    Dict(Arguments<'a>),
    Unary(&'a str, Wire<'a, TypedExpr>),
    Binary(&'a str, Wire<'a, TypedExpr>, Wire<'a, TypedExpr>),
    Block(LexicalArguments<'a>, Wire<'a, TypedExpr>),
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
        binding: Wire<'a, crate::binding::BindingId>,
        subject: Wire<'a, TypedExpr>,
        body: Wire<'a, TypedExpr>,
    },
    Failure(&'a str),
}
impl SerializeWithSources for ExprKind {
    fn serialize_with<S: Serializer>(&self, s: &Sources, serializer: S) -> Result<S::Ok, S::Error> {
        let out = match self {
            Self::Constant(v) => KindOut::Constant(v),
            Self::Input(v) => KindOut::Input(s.wire(v)),
            Self::Array(v) => KindOut::Array(s.wire(v.as_slice())),
            Self::Dict(v) => KindOut::Dict(Arguments(v, s)),
            Self::Unary(op, v) => KindOut::Unary(op, s.wire(&**v)),
            Self::Binary(op, a, b) => KindOut::Binary(op, s.wire(&**a), s.wire(&**b)),
            Self::Block(bindings, value) => {
                KindOut::Block(LexicalArguments(bindings, s), s.wire(&**value))
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
                binding: s.wire(binding),
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
    pattern: PatternWire<Wire<'a, crate::binding::BindingId>>,
    body: Wire<'a, TypedExpr>,
}
impl SerializeWithSources for ResultArm {
    fn serialize_with<S: Serializer>(
        &self,
        sources: &Sources,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        ArmOut {
            pattern: match &self.pattern {
                ResultPattern::Ok(id) => PatternWire::Ok(sources.wire(id)),
                ResultPattern::Err(id) => PatternWire::Err(sources.wire(id)),
                ResultPattern::Wildcard => PatternWire::Wildcard,
            },
            body: sources.wire(&self.body),
        }
        .serialize(serializer)
    }
}
#[derive(Serialize)]
struct InvocationOut<'a> {
    instance: Wire<'a, ResolvedInstance>,
    definition: &'a FunctionDef,
    arguments: CallArguments<'a>,
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
        if self.instance.function != self.definition().id.def_id() {
            return Err(S::Error::custom(
                "call declaration and instance identities differ",
            ));
        }
        InvocationOut {
            instance: sources.wire(&self.instance),
            definition: self.definition(),
            arguments: CallArguments(self, sources),
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
    locals: std::collections::BTreeMap<String, crate::binding::BindingId>,
    sources: Sources,
    namespace: SourceNamespace,
    allow_unknown: bool,
}
impl Input {
    fn declaration(
        &mut self,
        binding: &BindingIn,
        source: SourceId,
        extent: Option<TextRange>,
    ) -> Result<crate::binding::BindingId, SourceError> {
        if self.sources.name(source)? != binding.source
            || extent.is_some_and(|span| !binding.scope.contains_range(span))
        {
            return Err(SourceError::Binding(
                "lexical declaration does not match its structural source and scope".into(),
            ));
        }
        self.binding(binding, true)
    }
    fn binding(
        &mut self,
        binding: &BindingIn,
        declare: bool,
    ) -> Result<crate::binding::BindingId, SourceError> {
        if !binding.scope.contains_range(binding.span) {
            return Err(SourceError::Binding(
                "binding declaration lies outside its scope".into(),
            ));
        }
        let source = self.source(&binding.source)?;
        if let Some(id) = self.sources.bindings().find(
            &binding.name,
            source,
            binding.span,
            binding.scope,
            binding.owner.as_ref(),
        ) {
            return Ok(id);
        }
        if !self.allow_unknown || !declare {
            return Err(SourceError::Binding("unknown lexical declaration".into()));
        }
        let function = binding
            .owner
            .as_ref()
            .map(|name| {
                self.identity(crate::definition_identity::FunctionName {
                    module: name.module.clone(),
                    name: name.name.clone(),
                })
            })
            .transpose()?;
        let owner = if source == SourceId::DETACHED {
            Source::detached()
        } else {
            self.sources.source(source)?.clone()
        };
        self.sources
            .bindings_mut()
            .declare_with_owner(
                &binding.name,
                &owner,
                binding.span,
                binding.scope,
                function.as_ref(),
            )
            .map_err(|message| SourceError::Binding(message.into()))
    }
    fn reference(
        &mut self,
        binding: &BindingIn,
        source: SourceId,
        span: TextRange,
        known: bool,
    ) -> Result<crate::binding::BindingId, SourceError> {
        if self.sources.name(source)? != binding.source || !binding.scope.contains_range(span) {
            return Err(SourceError::Binding(
                "binding reference lies outside its source or scope".into(),
            ));
        }
        let id = self.binding(binding, known)?;
        match self.locals.get(&binding.name) {
            Some(visible) if *visible == id => Ok(id),
            None if known => Ok(id),
            _ => Err(SourceError::Binding(
                "dynamic free binding requires its explicit lexical environment".into(),
            )),
        }
    }
    fn identity(
        &mut self,
        name: crate::definition_identity::FunctionName,
    ) -> Result<crate::definitions::FunctionId, SourceError> {
        if let Some(id) = self.sources.definitions().find(&name.module, &name.name) {
            return self
                .sources
                .definitions()
                .function(id)
                .cloned()
                .map_err(SourceError::Definition);
        }
        if !self.allow_unknown {
            return Err(SourceError::InvalidDefinition(format!(
                "function `{}.{}` is not owned by this definition snapshot",
                name.module, name.name
            )));
        }
        let owner = crate::definitions::FunctionId::try_new(name.module, name.name)
            .map_err(|error| SourceError::InvalidDefinition(error.to_string()))?;
        self.sources.definitions_mut().insert(owner.clone());
        Ok(owner)
    }
    fn instance(&mut self, instance: InstanceIn) -> Result<ResolvedInstance, SourceError> {
        let id = self.identity(instance.function)?.def_id();
        Ok(ResolvedInstance {
            function: id,
            arguments: instance.arguments,
        })
    }
    fn definition(
        &mut self,
        definition: crate::definitions::FunctionDefWire<crate::definition_identity::FunctionName>,
    ) -> Result<FunctionDef, SourceError> {
        let definition = crate::definitions::FunctionDefWire {
            id: self.identity(definition.id)?,
            parameters: definition.parameters,
            children: definition.children,
            returns: definition.returns,
            comptime: definition.comptime,
            where_clause: definition.where_clause,
            children_type: definition.children_type,
            code: definition.code,
            implementation: definition.implementation,
            span: definition.span,
        };
        FunctionDef::try_from(definition)
            .map_err(|error| SourceError::InvalidDefinition(error.into()))
    }
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
    ctor: CtorIn,
    fields: Dict,
    children: Vec<NodeIn>,
    instance: Option<InstanceIn>,
}
#[derive(Deserialize)]
#[serde(untagged)]
enum CtorIn {
    Extension(ExtensionIn),
    Native(Ctor),
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtensionIn {
    #[serde(rename = "Extension")]
    extension: ExtensionCtorIn,
}
#[derive(Deserialize)]
struct ExtensionCtorIn {
    id: crate::definition_identity::FunctionName,
    accepts: crate::builtins::Accepts,
    level: Level,
}
#[derive(Deserialize)]
struct InstanceIn {
    function: crate::definition_identity::FunctionName,
    arguments: Vec<ComptimeValue>,
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
#[serde(deny_unknown_fields)]
struct BindingIn {
    #[serde(default)]
    owner: Option<crate::definition_identity::FunctionName>,
    name: String,
    source: String,
    #[serde(with = "crate::evaluation::span")]
    span: TextRange,
    #[serde(with = "crate::evaluation::span")]
    scope: TextRange,
}
#[derive(Deserialize)]
enum KindIn {
    Constant(ComptimeValue),
    Input(BindingIn),
    Array(Vec<ExpressionIn>),
    Dict(Vec<(String, ExpressionIn)>),
    Unary(String, Box<ExpressionIn>),
    Binary(String, Box<ExpressionIn>, Box<ExpressionIn>),
    Block(Vec<(BindingIn, ExpressionIn)>, Box<ExpressionIn>),
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
        binding: BindingIn,
        subject: Box<ExpressionIn>,
        body: Box<ExpressionIn>,
    },
    Failure(String),
}
#[derive(Deserialize)]
struct ArmIn {
    pattern: PatternWire<BindingIn>,
    body: ExpressionIn,
}
#[derive(Serialize, Deserialize)]
enum PatternWire<T> {
    Ok(T),
    Err(T),
    Wildcard,
}
#[derive(Deserialize)]
pub struct InvocationIn {
    instance: InstanceIn,
    definition: crate::definitions::FunctionDefWire<crate::definition_identity::FunctionName>,
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
                    ctor: match i.ctor {
                        CtorIn::Native(ctor) => ctor,
                        CtorIn::Extension(ctor) => {
                            let instance = s.instance(InstanceIn {
                                function: ctor.extension.id,
                                arguments: Vec::new(),
                            })?;
                            Ctor::Extension(crate::item::ExtensionCtor {
                                id: instance.function,
                                accepts: ctor.extension.accepts,
                                level: ctor.extension.level,
                            })
                        }
                    },
                    fields: i.fields,
                    children: Shared::from_nodes(
                        i.children
                            .into_iter()
                            .map(|n| Node::decode(n, s))
                            .collect::<Result<Vec<_>, _>>()?,
                    ),
                    instance: i
                        .instance
                        .map(|value| s.instance(value).map(Shared::new))
                        .transpose()?,
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
        let source = s.source(&v.source)?;
        let kind = match v.kind {
            KindIn::Constant(v) => ExprKind::Constant(v),
            KindIn::Input(binding) => {
                ExprKind::Input(s.reference(&binding, source, v.span, v.known.is_some())?)
            }
            KindIn::Array(v) => ExprKind::Array(expressions(v, s)?),
            KindIn::Dict(v) => ExprKind::Dict(arguments(v, s)?),
            KindIn::Unary(op, v) => ExprKind::Unary(op, expression(*v, s)?),
            KindIn::Binary(op, a, b) => {
                ExprKind::Binary(op, expression(*a, s)?, expression(*b, s)?)
            }
            KindIn::Block(bindings, tail) => {
                let previous = s.locals.clone();
                let result = (|| {
                    let mut values = Vec::new();
                    for (binding, value) in bindings {
                        let value = TypedExpr::decode(value, s)?;
                        let id = s.declaration(&binding, source, Some(v.span))?;
                        s.locals.insert(binding.name, id);
                        values.push((id, value));
                    }
                    Ok(ExprKind::Block(values, expression(*tail, s)?))
                })();
                s.locals = previous;
                result?
            }
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
                        let previous = s.locals.clone();
                        let result = (|| {
                            let is_ok = matches!(&v.pattern, PatternWire::Ok(_));
                            let pattern = match v.pattern {
                                PatternWire::Ok(binding) | PatternWire::Err(binding) => {
                                    let extent =
                                        (v.body.source == binding.source).then_some(v.body.span);
                                    let id = s.declaration(&binding, source, extent)?;
                                    s.locals.insert(binding.name, id);
                                    id
                                }
                                PatternWire::Wildcard => {
                                    return Ok(ResultArm {
                                        pattern: ResultPattern::Wildcard,
                                        body: TypedExpr::decode(v.body, s)?,
                                    });
                                }
                            };
                            Ok(ResultArm {
                                pattern: if is_ok {
                                    ResultPattern::Ok(pattern)
                                } else {
                                    ResultPattern::Err(pattern)
                                },
                                body: TypedExpr::decode(v.body, s)?,
                            })
                        })();
                        s.locals = previous;
                        result
                    })
                    .collect::<Result<_, SourceError>>()?,
            },
            KindIn::Call(v) => ExprKind::Call(Invocation::decode(v, s)?.into()),
            KindIn::Content(v) => ExprKind::Content(Shared::from_nodes(
                v.into_iter()
                    .map(|v| Node::decode(v, s))
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            KindIn::For {
                binding,
                subject,
                body,
            } => {
                let subject = TypedExpr::decode(*subject, s)?.into();
                let id = s.declaration(&binding, source, Some(v.span))?;
                let previous = s.locals.insert(binding.name.clone(), id);
                let result = TypedExpr::decode(*body, s);
                match previous {
                    Some(id) => {
                        s.locals.insert(binding.name, id);
                    }
                    None => {
                        s.locals.remove(&binding.name);
                    }
                }
                ExprKind::For {
                    binding: id,
                    subject,
                    body: result?.into(),
                }
            }
            KindIn::Failure(v) => ExprKind::Failure(v),
        };
        Ok(Self {
            ty: v.ty,
            known: v.known,
            kind,
            span: v.span,
            source,
            shape: v.shape,
        })
    }
}
impl DeserializeWithSources for Invocation {
    type Input = InvocationIn;
    fn decode(v: InvocationIn, s: &mut Input) -> Result<Self, SourceError> {
        let instance = s.instance(v.instance)?;
        let definition = s.definition(v.definition)?;
        if instance.function != definition.id.def_id() {
            return Err(SourceError::InvalidDefinition(
                "call declaration and instance identities differ".into(),
            ));
        }
        let mut call = Self::new(
            instance,
            definition,
            arguments(v.arguments, s)?
                .into_iter()
                .map(|(name, value)| (name.into(), value))
                .collect(),
        );
        call.children = v.children.map(|v| expression(*v, s)).transpose()?;
        call.attrs = v.attrs;
        call.span = v.span;
        call.source = s.source(&v.source)?;
        call.site = v.site;
        call.resource_root = v.resource_root;
        call.invalid = v.invalid;
        Ok(call)
    }
}
