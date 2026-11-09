//! Source identities and owned, immutable source revisions. No filesystem IO.
//!
//! Nodes carry only an ID. An owning `Source`/`Sources` must travel with any
//! retained IR. The registry holds weak references to identities, never bodies;
//! the last owner removes its registry entry, including the key/name storage.
use crate::identity::{Id, IdAllocator};
use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    sync::{Arc, LazyLock, Mutex, Weak},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum SourceTag {}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceId(Id<SourceTag>);

impl SourceId {
    /// A synthetic location with no source. Never resolved to another source.
    pub const DETACHED: Self = Self(Id::new(1));
}
impl Default for SourceId {
    fn default() -> Self {
        Self::DETACHED
    }
}

/// A host-declared, stable logical domain, not a resource grant or a revision.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceNamespace(Arc<str>);
impl SourceNamespace {
    pub fn named(name: impl Into<Arc<str>>) -> Self {
        Self(name.into())
    }
    pub fn name(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    namespace: SourceNamespace,
    name: Arc<str>,
}

struct Identity {
    id: SourceId,
    key: Key,
    named: bool,
    bindings: Arc<Mutex<crate::binding::BindingRegistry>>,
}
static NEXT: IdAllocator<SourceTag> = IdAllocator::new(2);
static IDENTITIES: LazyLock<Mutex<HashMap<Key, Weak<Identity>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn allocate() -> Result<SourceId, SourceError> {
    NEXT.allocate().map(SourceId).ok_or(SourceError::Exhausted)
}
impl Drop for Identity {
    fn drop(&mut self) {
        if self.named {
            let mut entries = IDENTITIES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // A new identity may already have replaced an expired weak entry.
            if entries
                .get(&self.key)
                .is_some_and(|w| std::ptr::eq(w.as_ptr(), self))
            {
                entries.remove(&self.key);
            }
        }
    }
}

struct Revision {
    identity: Arc<Identity>,
    text: Option<Arc<str>>,
}

/// An owned source revision. Clones share both identity and exact consumed text.
#[derive(Clone)]
pub struct Source(Arc<Revision>);
impl Source {
    pub(crate) fn binding_registry(&self) -> Arc<Mutex<crate::binding::BindingRegistry>> {
        self.0.identity.bindings.clone()
    }
    pub fn detached() -> Self {
        Self(Arc::new(Revision {
            identity: Arc::new(Identity {
                bindings: Default::default(),
                id: SourceId::DETACHED,
                key: Key {
                    namespace: SourceNamespace::default(),
                    name: "".into(),
                },
                named: false,
            }),
            text: None,
        }))
    }
    pub fn new(name: impl Into<Arc<str>>, text: impl Into<Arc<str>>) -> Result<Self, SourceError> {
        Self::in_namespace(SourceNamespace::default(), name, Some(text.into()))
    }
    pub fn metadata(name: impl Into<Arc<str>>) -> Result<Self, SourceError> {
        Self::in_namespace(SourceNamespace::default(), name, None)
    }
    pub fn in_namespace(
        namespace: SourceNamespace,
        name: impl Into<Arc<str>>,
        text: Option<Arc<str>>,
    ) -> Result<Self, SourceError> {
        let key = Key {
            namespace,
            name: name.into(),
        };
        if key.name.is_empty() {
            return Err(SourceError::EmptyName);
        }
        let identity = {
            let mut entries = IDENTITIES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(existing) = entries.get(&key).and_then(Weak::upgrade) {
                existing
            } else {
                let identity = Arc::new(Identity {
                    bindings: Default::default(),
                    id: allocate()?,
                    key: key.clone(),
                    named: true,
                });
                entries.insert(key, Arc::downgrade(&identity));
                identity
            }
        };
        Ok(Self(Arc::new(Revision { identity, text })))
    }
    pub fn anonymous(
        namespace: SourceNamespace,
        display_name: impl Into<Arc<str>>,
        text: Option<Arc<str>>,
    ) -> Result<Self, SourceError> {
        let name = display_name.into();
        if name.is_empty() {
            return Err(SourceError::EmptyName);
        }
        Ok(Self(Arc::new(Revision {
            identity: Arc::new(Identity {
                bindings: Default::default(),
                id: allocate()?,
                key: Key { namespace, name },
                named: false,
            }),
            text,
        })))
    }
    pub fn id(&self) -> SourceId {
        self.0.identity.id
    }
    pub fn name(&self) -> &str {
        &self.0.identity.key.name
    }
    pub fn namespace(&self) -> &SourceNamespace {
        &self.0.identity.key.namespace
    }
    pub fn text(&self) -> Option<&Arc<str>> {
        self.0.text.as_ref()
    }
    pub fn is_anonymous(&self) -> bool {
        !self.0.identity.named
    }
    pub fn same_revision(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.id() == other.id() && self.text() == other.text()
    }
}
impl fmt::Debug for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Source")
            .field("namespace", self.namespace())
            .field("name", &self.name())
            .field("has_text", &self.text().is_some())
            .finish()
    }
}
impl PartialEq for Source {
    fn eq(&self, other: &Self) -> bool {
        self.same_revision(other)
    }
}
impl Eq for Source {}
impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
impl std::ops::Deref for Source {
    type Target = str;
    fn deref(&self) -> &str {
        self.name()
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Source {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Source {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = <String as serde::Deserialize>::deserialize(deserializer)?;
        Self::metadata(name).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    EmptyName,
    Exhausted,
    Missing(SourceId),
    MissingText(SourceId),
    Conflict { name: String },
    Ambiguous { name: String },
    Unknown { name: String },
    Definition(crate::definition_identity::DefinitionError),
    InvalidDefinition(String),
    Binding(String),
}
impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => {
                f.write_str("empty source names are reserved for detached locations")
            }
            Self::Exhausted => f.write_str("32-bit source identity space exhausted"),
            Self::Missing(id) => write!(f, "source {id:?} is not owned by this source snapshot"),
            Self::MissingText(id) => write!(f, "source {id:?} has no retained text"),
            Self::Conflict { name } => write!(f, "incompatible source revisions for `{name}`"),
            Self::Ambiguous { name } => write!(
                f,
                "ambiguous source name `{name}`; use its identity or namespace"
            ),
            Self::Unknown { name } => write!(f, "unknown source name `{name}`"),
            Self::Definition(error) => error.fmt(f),
            Self::InvalidDefinition(error) => f.write_str(error),
            Self::Binding(error) => f.write_str(error),
        }
    }
}
impl std::error::Error for SourceError {}

/// An immutable-view, copy-on-write collection of owned source revisions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sources(
    Arc<BTreeMap<SourceId, Source>>,
    crate::definition_identity::Definitions,
    crate::binding::Bindings,
);
impl Sources {
    pub fn bindings(&self) -> &crate::binding::Bindings {
        &self.2
    }
    pub fn bindings_mut(&mut self) -> &mut crate::binding::Bindings {
        &mut self.2
    }
    pub fn definitions(&self) -> &crate::definition_identity::Definitions {
        &self.1
    }
    pub fn definitions_mut(&mut self) -> &mut crate::definition_identity::Definitions {
        &mut self.1
    }
    pub fn with_definitions(
        mut self,
        definitions: &crate::definition_identity::Definitions,
    ) -> Self {
        self.1.merge(definitions);
        self
    }
    /// Verify all locations, including deferred IR and expansion chains.
    pub fn validate_node(&self, node: &crate::item::Node) -> Result<(), SourceError> {
        self.validate_references(Reference::Node(node))
    }
    pub fn validate_expression(
        &self,
        expression: &crate::evaluation::TypedExpr,
    ) -> Result<(), SourceError> {
        self.validate_references(Reference::Expression(expression))
    }
    pub fn validate_program(
        &self,
        program: &crate::evaluation::CheckedProgram,
    ) -> Result<(), SourceError> {
        for record in program.instances.values() {
            if record.inputs.parameters.len() > record.definition().parameters.len() {
                return Err(SourceError::Binding(
                    "checked parameter bindings exceed the declaration".into(),
                ));
            }
            for (name, id) in record.named_inputs() {
                self.validate_binding(id)?;
                if self
                    .2
                    .get(id)
                    .map_err(|message| SourceError::Binding(message.into()))?
                    .name
                    != name
                {
                    return Err(SourceError::Binding(
                        "checked parameter name and binding identity differ".into(),
                    ));
                }
            }
            if let Some(body) = &record.body {
                self.validate_expression(body)?;
            }
        }
        Ok(())
    }
    pub(crate) fn validate_binding(
        &self,
        id: crate::binding::BindingId,
    ) -> Result<(), SourceError> {
        let info = self
            .2
            .get(id)
            .map_err(|message| SourceError::Binding(message.into()))?;
        if info.source != SourceId::DETACHED {
            self.source(info.source)?;
        }
        Ok(())
    }
    fn validate_references(&self, root: Reference<'_>) -> Result<(), SourceError> {
        use crate::evaluation::ExprKind;
        let check = |id| {
            if id == SourceId::DETACHED {
                Ok(())
            } else {
                self.source(id).map(|_| ())
            }
        };
        let mut pending = vec![root];
        while let Some(reference) = pending.pop() {
            #[cfg(test)]
            tests::record_visit(&reference);
            match reference {
                Reference::Node(node) => {
                    if let Some(item) = node.as_item() {
                        if let Some(instance) = &item.instance {
                            self.1
                                .function(instance.function)
                                .map_err(SourceError::Definition)?;
                        }
                        if let crate::item::Ctor::Extension(extension) = &item.ctor
                            && !item
                                .instance
                                .as_ref()
                                .is_some_and(|instance| instance.function == extension.id)
                        {
                            self.1
                                .function(extension.id)
                                .map_err(SourceError::Definition)?;
                        }
                    }
                    if let Some(origin) = &node.origin {
                        check(origin.source)?;
                        for location in origin.expansion.iter() {
                            check(location.source)?;
                        }
                    }
                    // `children()` projects literal content out of expression IR.
                    // Walking both views would visit the same nodes repeatedly.
                    if let Some(expression) = node.as_expr() {
                        pending.push(Reference::Expression(expression));
                    } else {
                        pending.extend(node.children().iter().map(Reference::Node));
                    }
                }
                Reference::Expression(expression) => {
                    check(expression.source)?;
                    match &expression.kind {
                        ExprKind::Array(values) => {
                            pending.extend(values.iter().map(Reference::Expression))
                        }
                        ExprKind::Dict(values) => pending
                            .extend(values.iter().map(|(_, value)| Reference::Expression(value))),
                        ExprKind::Unary(_, value)
                        | ExprKind::Some(value)
                        | ExprKind::Unwrap(value)
                        | ExprKind::Ok(value)
                        | ExprKind::Err(value) => pending.push(Reference::Expression(value)),
                        ExprKind::Binary(_, a, b) => {
                            pending.push(Reference::Expression(a));
                            pending.push(Reference::Expression(b));
                        }
                        ExprKind::Block(values, value) => {
                            for (id, _) in values {
                                self.validate_binding(*id)?;
                            }
                            pending.extend(
                                values.iter().map(|(_, value)| Reference::Expression(value)),
                            );
                            pending.push(Reference::Expression(value));
                        }
                        ExprKind::If(a, b, c) => {
                            pending.push(Reference::Expression(a));
                            pending.push(Reference::Expression(b));
                            pending.push(Reference::Expression(c));
                        }
                        ExprKind::MatchResult { subject, arms } => {
                            for arm in arms {
                                if let crate::evaluation::ResultPattern::Ok(id)
                                | crate::evaluation::ResultPattern::Err(id) = arm.pattern
                                {
                                    self.validate_binding(id)?;
                                }
                            }
                            pending.push(Reference::Expression(subject));
                            pending.extend(arms.iter().map(|arm| Reference::Expression(&arm.body)));
                        }
                        ExprKind::Call(call) => {
                            self.1
                                .function(call.instance.function)
                                .map_err(SourceError::Definition)?;
                            if call.definition().id.def_id() != call.instance.function {
                                return Err(SourceError::InvalidDefinition(
                                    "call declaration and instance identities differ".into(),
                                ));
                            }
                            check(call.source)?;
                            pending.extend(
                                call.arguments()
                                    .map(|(_, value)| Reference::Expression(value)),
                            );
                            if let Some(children) = &call.children {
                                pending.push(Reference::Expression(children));
                            }
                        }
                        ExprKind::Content(nodes) => {
                            pending.extend(nodes.iter().map(Reference::Node))
                        }
                        ExprKind::For {
                            binding,
                            subject,
                            body,
                        } => {
                            self.validate_binding(*binding)?;
                            pending.push(Reference::Expression(subject));
                            pending.push(Reference::Expression(body));
                        }
                        ExprKind::Input(id) => {
                            self.validate_binding(*id)?;
                        }
                        ExprKind::Constant(_) | ExprKind::Failure(_) => {}
                    }
                }
            }
        }
        Ok(())
    }
    pub fn new() -> Self {
        Self::default()
    }
    pub fn source(&self, id: SourceId) -> Result<&Source, SourceError> {
        self.0.get(&id).ok_or(SourceError::Missing(id))
    }
    pub fn name(&self, id: SourceId) -> Result<&str, SourceError> {
        if id == SourceId::DETACHED {
            return Ok("");
        }
        Ok(self.source(id)?.name())
    }
    pub fn text(&self, id: SourceId) -> Result<&Arc<str>, SourceError> {
        self.source(id)?.text().ok_or(SourceError::MissingText(id))
    }
    /// Compatibility for host entry points already addressing a unique name.
    /// ID-bearing IR must use `source`/`text`, not stringify an ID for lookup.
    pub fn get(&self, name: &str) -> Option<&Arc<str>> {
        self.by_name(name).ok()?.text()
    }
    pub fn by_name(&self, name: &str) -> Result<&Source, SourceError> {
        let mut matches = self.0.values().filter(|s| s.name() == name);
        let first = matches
            .next()
            .ok_or_else(|| SourceError::Unknown { name: name.into() })?;
        if matches.next().is_some() {
            return Err(SourceError::Ambiguous { name: name.into() });
        }
        Ok(first)
    }
    pub fn by_namespace(
        &self,
        namespace: &SourceNamespace,
        name: &str,
    ) -> Result<&Source, SourceError> {
        let mut matches = self
            .0
            .values()
            .filter(|s| s.namespace() == namespace && s.name() == name);
        let first = matches
            .next()
            .ok_or_else(|| SourceError::Unknown { name: name.into() })?;
        if matches.next().is_some() {
            return Err(SourceError::Ambiguous { name: name.into() });
        }
        Ok(first)
    }
    pub fn insert(&mut self, source: Source) -> Result<SourceId, SourceError> {
        let id = source.id();
        if let Some(previous) = self.0.get(&id) {
            if previous.same_revision(&source) || source.text().is_none() {
                return Ok(id);
            }
            if previous.text().is_some() {
                return Err(SourceError::Conflict {
                    name: source.name().into(),
                });
            }
        }
        Arc::make_mut(&mut self.0).insert(id, source);
        Ok(id)
    }
    /// Atomic merge: no partial changes if any revision conflicts.
    pub fn merge(&mut self, other: &Self) -> Result<(), SourceError> {
        // Name ownership is independent of source revision compatibility; merge
        // only after all conflicts have been checked below.
        if Arc::ptr_eq(&self.0, &other.0) || other.is_empty() {
            self.1.merge(&other.1);
            self.2.merge(&other.2);
            return Ok(());
        }
        for (id, source) in other.0.iter() {
            if let Some(previous) = self.0.get(id)
                && previous.text().is_some()
                && source.text().is_some()
                && !previous.same_revision(source)
            {
                return Err(SourceError::Conflict {
                    name: source.name().into(),
                });
            }
        }
        for source in other.0.values() {
            self.insert(source.clone())?;
        }
        self.1.merge(&other.1);
        self.2.merge(&other.2);
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Deterministic external order, not process-local ID allocation order.
    pub fn records(&self) -> impl Iterator<Item = &Source> {
        let mut records: Vec<_> = self.0.values().collect();
        records.sort_by(|a, b| (a.name(), a.namespace()).cmp(&(b.name(), b.namespace())));
        records.into_iter()
    }
    /// Legacy path/text projection, available only for unambiguous names.
    pub fn texts(&self) -> Result<BTreeMap<&str, &Arc<str>>, SourceError> {
        let mut names = BTreeMap::new();
        for source in self.records() {
            self.by_name(source.name())?;
            if let Some(text) = source.text()
                && names.insert(source.name(), text).is_some()
            {
                return Err(SourceError::Ambiguous {
                    name: source.name().into(),
                });
            }
        }
        Ok(names)
    }
}
impl From<Source> for Sources {
    fn from(source: Source) -> Self {
        Self(
            Arc::new(BTreeMap::from([(source.id(), source)])),
            Default::default(),
            Default::default(),
        )
    }
}
enum Reference<'a> {
    Node(&'a crate::item::Node),
    Expression(&'a crate::evaluation::TypedExpr),
}

/// An IR value and the source snapshot required to interpret its locations.
#[derive(Debug, Clone, PartialEq)]
pub struct Located<T> {
    pub value: T,
    pub sources: Sources,
}
impl<T> Located<T> {
    pub fn new(value: T, sources: Sources) -> Self {
        Self { value, sources }
    }
    pub fn as_ref(&self) -> Located<&T> {
        Located::new(&self.value, self.sources.clone())
    }
    pub fn into_parts(self) -> (T, Sources) {
        (self.value, self.sources)
    }
}
impl<T> std::ops::Deref for Located<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<T> std::ops::DerefMut for Located<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

/// A self-contained diagnostic/read location. Low-frequency errors may escape
/// compilation independently, so they pin the exact source revision explicitly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    pub source: Source,
    pub start: u32,
    pub end: u32,
}
impl SourceSpan {
    pub fn location(&self) -> crate::item::SourceLocation {
        crate::item::SourceLocation {
            source: self.source.id(),
            start: self.start,
            end: self.end,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        builtins::Accepts,
        definitions::{FunctionDef, FunctionId, FunctionInstance, Type},
        evaluation::{ExprKind, Invocation, TypedExpr},
        item::{Ctor, Node, Origin, SourceLocation},
    };

    thread_local! {
        static VISITS: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    }

    pub(super) fn record_visit(reference: &Reference<'_>) {
        VISITS.with(|visits| {
            let (nodes, expressions) = visits.get();
            visits.set(match reference {
                Reference::Node(_) => (nodes + 1, expressions),
                Reference::Expression(_) => (nodes, expressions + 1),
            });
        });
    }

    fn expression(kind: ExprKind, source: SourceId) -> TypedExpr {
        TypedExpr {
            ty: None,
            known: None,
            kind,
            span: Default::default(),
            source,
            shape: Default::default(),
        }
    }

    fn call(children: TypedExpr, source: SourceId) -> Node {
        let definition = FunctionDef::new(
            FunctionId::new("notist", "strong"),
            Accepts::Inline,
            Type::InlineContent,
        );
        let mut call = Invocation::new(
            FunctionInstance {
                function: definition.id.clone(),
                arguments: Vec::new(),
            }
            .into(),
            definition,
            Vec::new(),
        );
        call.children = Some(Box::new(children));
        call.source = source;
        Node::expression(expression(ExprKind::Call(call.into()), source))
    }

    #[test]
    fn nested_literal_calls_visit_each_stored_node_and_expression_once() {
        let source = Source::metadata("walk-once.not").unwrap();
        let sources = Sources::from(source.clone())
            .with_definitions(crate::builtins::registry().identities());
        let mut node = Node::text("leaf".into(), Default::default());
        for _ in 0..12 {
            let children = expression(ExprKind::Content(vec![node].into()), source.id());
            node = call(children, source.id());
            // The content view and the IR borrow the very same child node.
            let ExprKind::Content(children) = &node.call().unwrap().children.as_ref().unwrap().kind
            else {
                unreachable!()
            };
            assert!(std::ptr::eq(&node.children()[0], &children[0]));
        }
        let root = Node::new(Ctor::Group, Default::default()).with_children(vec![node]);
        VISITS.with(|visits| visits.set((0, 0)));
        sources.validate_node(&root).unwrap();
        VISITS.with(|visits| assert_eq!(visits.get(), (14, 24)));
    }

    #[test]
    fn exclusive_traversal_still_checks_nested_origins_expansions_and_call_sources() {
        let owner = Source::metadata("walk-owner.not").unwrap();
        let missing = Source::metadata("walk-missing.not").unwrap();
        let sources =
            Sources::from(owner.clone()).with_definitions(crate::builtins::registry().identities());
        for position in 0..6 {
            let mut leaf = Node::text("leaf".into(), Default::default());
            leaf.origin = Some(Origin {
                source: if position == 0 {
                    missing.id()
                } else {
                    owner.id()
                },
                start: 0,
                end: 0,
                expansion: if position == 1 {
                    vec![SourceLocation {
                        source: missing.id(),
                        start: 0,
                        end: 0,
                    }]
                    .into()
                } else {
                    Default::default()
                },
            });
            let content = expression(
                ExprKind::Content(vec![leaf].into()),
                if position == 2 {
                    missing.id()
                } else {
                    owner.id()
                },
            );
            let mut node = call(content, owner.id());
            if position == 3 {
                node.as_expr_mut().unwrap().source = missing.id();
            }
            if position == 4 {
                node.call_mut().unwrap().source = missing.id();
            }
            if position == 5 {
                node.origin.as_mut().unwrap().source = missing.id();
            }
            let root = Node::new(Ctor::Group, Default::default()).with_children(vec![node]);
            assert!(
                matches!(sources.validate_node(&root), Err(SourceError::Missing(id)) if id == missing.id()),
                "position {position}"
            );
        }
    }

    #[test]
    fn exclusive_traversal_still_checks_non_content_expression_branches() {
        let owner = Source::metadata("walk-branches-owner.not").unwrap();
        let missing = Source::metadata("walk-branches-missing.not").unwrap();
        let mut sources =
            Sources::from(owner.clone()).with_definitions(crate::builtins::registry().identities());
        let binding = sources
            .bindings_mut()
            .declare("value", &owner, Default::default())
            .unwrap();
        for position in 0..5 {
            let input = |index| {
                expression(
                    ExprKind::Input(binding),
                    if position == index {
                        missing.id()
                    } else {
                        owner.id()
                    },
                )
            };
            let condition = expression(
                ExprKind::If(Box::new(input(0)), Box::new(input(1)), Box::new(input(2))),
                owner.id(),
            );
            let body = expression(
                ExprKind::For {
                    binding,
                    subject: input(3).into(),
                    body: input(4).into(),
                },
                owner.id(),
            );
            let mut node = call(body, owner.id());
            node.call_mut()
                .unwrap()
                .set_arguments(vec![("argument".into(), condition)]);
            assert!(node.children().is_empty());
            assert!(
                matches!(sources.validate_node(&node), Err(SourceError::Missing(id)) if id == missing.id()),
                "position {position}"
            );
        }
    }

    #[test]
    fn identity_reclaims_its_key_and_never_reuses_a_live_or_retired_id() {
        let first = Source::new("reclaim-test.not", "old").unwrap();
        let second = Source::new("reclaim-test.not", "new").unwrap();
        let id = first.id();
        assert_eq!(second.id(), id);
        let weak = Arc::downgrade(&first.0.identity);
        let key = first.0.identity.key.clone();
        drop(first);
        assert!(weak.upgrade().is_some());
        drop(second);
        assert!(weak.upgrade().is_none());
        assert!(!IDENTITIES.lock().unwrap().contains_key(&key));
        let next = Source::new("reclaim-test.not", "again").unwrap();
        assert_ne!(id, next.id());
    }
    #[test]
    fn conflicts_are_atomic_and_old_snapshots_keep_their_text() {
        let old = Source::new("version-test.not", "old").unwrap();
        let new = Source::new("version-test.not", "new").unwrap();
        let mut a = Sources::from(old.clone());
        let before = a.clone();
        assert!(a.merge(&Sources::from(new.clone())).is_err());
        assert_eq!(a, before);
        assert_eq!(a.text(old.id()).unwrap().as_ref(), "old");
        assert_eq!(
            Sources::from(new.clone()).text(new.id()).unwrap().as_ref(),
            "new"
        );
    }
    #[test]
    fn namespaces_anonymous_sources_and_missing_text_are_explicit() {
        let a = Source::in_namespace(SourceNamespace::named("a"), "same.not", Some("a".into()))
            .unwrap();
        let b = Source::in_namespace(SourceNamespace::named("b"), "same.not", Some("b".into()))
            .unwrap();
        assert_ne!(a.id(), b.id());
        let mut sources = Sources::from(a.clone());
        sources.insert(b).unwrap();
        assert!(matches!(
            sources.by_name("same.not"),
            Err(SourceError::Ambiguous { .. })
        ));
        assert!(sources.texts().is_err());
        let x = Source::anonymous(SourceNamespace::default(), "stdin", None).unwrap();
        let y = Source::anonymous(SourceNamespace::default(), "stdin", None).unwrap();
        assert_ne!(x.id(), y.id());
        assert!(matches!(
            Sources::from(x.clone()).text(x.id()),
            Err(SourceError::MissingText(_))
        ));
    }
    #[test]
    fn ids_are_32_bit_and_do_not_have_a_16_bit_source_limit() {
        assert_eq!(size_of::<SourceId>(), 4);
        assert_eq!(size_of::<Option<SourceId>>(), 4);
        assert_eq!(format!("{:?}", SourceId::DETACHED), "SourceId(1)");
        let sources: Vec<_> = (0..70_000)
            .map(|i| Source::metadata(format!("wide-{i}.not")).unwrap())
            .collect();
        assert_ne!(sources[0].id(), sources[69_999].id());
    }
    #[test]
    fn concurrent_registration_shares_a_retained_identity() {
        let source = Source::metadata("concurrent-source.not").unwrap();
        let handles: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(|| Source::metadata("concurrent-source.not").unwrap()))
            .collect();
        for handle in handles {
            assert_eq!(handle.join().unwrap().id(), source.id());
        }
    }
}
