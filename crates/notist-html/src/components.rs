//! Signature-bound HTML targets and the versioned component parameter protocol.
use notist_core::{
    builtins,
    definitions::{FunctionDef, FunctionId},
    item::{Ctor, Item, Value},
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// External function identities actually visited by rendering an evaluated tree.
/// Errors hide their children; expanded Code boundaries render only their result.
pub fn component_ids(root: &Item) -> BTreeSet<notist_core::definitions::DefId> {
    let mut ids = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(item) = stack.pop() {
        if item.is_error() {
            continue;
        }
        if let Some(id) = item.function_id() {
            ids.insert(id);
        }
        stack.extend(item.children().iter());
    }
    ids
}

/// Resource identity and published browser URL have different host semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleLocator {
    Resource(PathBuf),
    Url(String),
}
impl ModuleLocator {
    pub fn resource(&self) -> Option<&Path> {
        if let Self::Resource(path) = self {
            Some(path)
        } else {
            None
        }
    }
    pub fn url(&self) -> Option<&str> {
        if let Self::Url(url) = self {
            Some(url)
        } else {
            None
        }
    }
    fn is_empty(&self) -> bool {
        match self {
            Self::Resource(path) => path.as_os_str().is_empty(),
            Self::Url(url) => url.is_empty(),
        }
    }
}
impl From<String> for ModuleLocator {
    fn from(url: String) -> Self {
        Self::Url(url)
    }
}
impl From<&str> for ModuleLocator {
    fn from(url: &str) -> Self {
        Self::Url(url.into())
    }
}
impl From<&String> for ModuleLocator {
    fn from(url: &String) -> Self {
        Self::Url(url.clone())
    }
}

/// Relative candidates, in the module's logical resource root. Neither has priority.
pub fn component_entries(name: &str) -> [PathBuf; 2] {
    [
        PathBuf::from(format!("{name}.js")),
        PathBuf::from(name).join("index.js"),
    ]
}
/// Validate availability without IO; access failures must be handled by the caller.
pub fn select_component_entry(name: &str, available: [bool; 2]) -> Result<Option<PathBuf>, String> {
    let entries = component_entries(name);
    match available {
        [true, true] => Err(format!(
            "conflicting component entries: `{}` and `{}`",
            entries[0].display(),
            entries[1].display()
        )),
        [true, false] => Ok(Some(entries[0].clone())),
        [false, true] => Ok(Some(entries[1].clone())),
        [false, false] => Ok(None),
    }
}

#[derive(Debug, Clone)]
pub struct BindingError {
    pub id: notist_core::definitions::DefId,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct Component {
    pub(crate) signature: Arc<notist_core::signature::ResolvedSignature>,
    pub id: FunctionId,
    pub tag: String,
    /// Browser module locator interpreted by the host, not the renderer.
    pub module: ModuleLocator,
    pub definition: Arc<FunctionDef>,
}
impl Component {
    /// A possible shell, not an installed/validated host binding.
    pub fn candidate(
        signature: Arc<notist_core::signature::ResolvedSignature>,
        module: ModuleLocator,
    ) -> Self {
        let definition = Arc::new(signature.declaration().clone());
        Self {
            id: definition.id.clone(),
            tag: component_tag(&definition.id),
            module,
            definition,
            signature,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Target {
    Native(Ctor),
    Component(Component),
}

#[derive(Debug, Clone)]
pub struct HtmlRegistry {
    targets: Arc<BTreeMap<notist_core::definitions::DefId, Target>>,
    identities: notist_core::definition_identity::Definitions,
}
impl Default for HtmlRegistry {
    fn default() -> Self {
        static NATIVE: OnceLock<HtmlRegistry> = OnceLock::new();
        let targets = NATIVE.get_or_init(|| {
            let mut registry = Self {
                targets: Arc::new(BTreeMap::new()),
                identities: Default::default(),
            };
            for definition in builtins::registry().functions() {
                let Some(ctor) = Ctor::from_name(&definition.id.name) else {
                    continue;
                };
                registry
                    .register(definition, Target::Native(ctor))
                    .expect("valid native HTML target");
            }
            registry
        });
        targets.clone()
    }
}
impl HtmlRegistry {
    /// Bind all targets using the authoritative semantic registry. Errors do
    /// not return a partially installed HTML environment; declarations remain valid.
    pub fn from_bindings(
        registry: &notist_core::registry::Registry,
        bindings: &BTreeMap<notist_core::definitions::DefId, ModuleLocator>,
    ) -> Result<Self, Vec<BindingError>> {
        let mut html = Self::default();
        let mut errors = Vec::new();
        let mut ordered: Vec<_> = bindings.iter().collect();
        ordered.sort_by(|(a, _), (b, _)| {
            registry
                .identities()
                .function(**a)
                .ok()
                .cmp(&registry.identities().function(**b).ok())
        });
        for (id, module) in ordered {
            let result = registry
                .definition(*id)
                .ok_or_else(|| "unknown component definition identity".to_string())
                .and_then(|definition| html.bind_component(definition, module.clone()));
            if let Err(message) = result {
                errors.push(BindingError { id: *id, message });
            }
        }
        if errors.is_empty() {
            Ok(html)
        } else {
            Err(errors)
        }
    }
    pub fn get(&self, id: &FunctionId) -> Option<&Target> {
        self.target(id.def_id())
    }
    pub fn target(&self, id: notist_core::definitions::DefId) -> Option<&Target> {
        self.targets.get(&id)
    }
    pub fn identities(&self) -> &notist_core::definition_identity::Definitions {
        &self.identities
    }
    pub fn bind_component(
        &mut self,
        definition: &FunctionDef,
        module: impl Into<ModuleLocator>,
    ) -> Result<(), String> {
        let owned = Arc::new(definition.clone());
        self.register(
            &owned,
            Target::Component(Component {
                signature: Arc::new(
                    notist_core::signature::ResolvedSignature::from_shared(owned.clone())
                        .map_err(str::to_owned)?,
                ),
                id: definition.id.clone(),
                tag: component_tag(&definition.id),
                module: module.into(),
                definition: owned.clone(),
            }),
        )
    }
    /// Both native handlers and components enter the same identity registry.
    pub fn register(&mut self, definition: &FunctionDef, target: Target) -> Result<(), String> {
        let mut module = notist_core::definitions::DefinitionModule::new(&definition.id.module);
        module.functions.push(definition.clone());
        let errors = notist_core::definitions::validate_module(&module);
        if !errors.is_empty() {
            return Err(errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("; "));
        }
        if self.targets.contains_key(&definition.id.def_id()) {
            return Err(format!(
                "HTML target already registered for `{}`",
                definition.id
            ));
        }
        if let Target::Component(component) = &target {
            if component.id != definition.id || component.tag != component_tag(&definition.id) {
                return Err("component identity mismatch".into());
            }
            if component.module.is_empty() {
                return Err("component module cannot be empty".into());
            }
            if [
                "annotation-xml",
                "color-profile",
                "font-face",
                "font-face-src",
                "font-face-uri",
                "font-face-format",
                "font-face-name",
                "missing-glyph",
            ]
            .contains(&component.tag.as_str())
            {
                return Err(format!("reserved custom element tag `{}`", component.tag));
            }
            if self.targets.values().any(|target| matches!(target, Target::Component(existing) if existing.tag == component.tag)) {
                return Err(format!("component tag collision: `{}`", component.tag));
            }
            let mut attributes = BTreeSet::from([
                "notist-protocol".to_owned(),
                "notist-content".to_owned(),
                "notist-function".to_owned(),
            ]);
            for parameter in &definition.comptime {
                let attribute = format!("notist-comptime-{}", encode_name(&parameter.name));
                if !attributes.insert(attribute.clone()) {
                    return Err(format!(
                        "component parameter attribute collision: `{attribute}`"
                    ));
                }
            }
            for parameter in &definition.parameters {
                let attribute = parameter_attribute(&parameter.name);
                if !attributes.insert(attribute.clone()) {
                    return Err(format!(
                        "component parameter attribute collision: `{attribute}`"
                    ));
                }
            }
        }
        let target = match target {
            Target::Component(mut component) => {
                // The supplied semantic definition is authoritative; target records
                // never introduce their own signature, including NaN defaults.
                if !std::ptr::eq(component.signature.declaration(), definition)
                    || !std::ptr::eq(component.definition.as_ref(), definition)
                {
                    let owned = Arc::new(definition.clone());
                    component.signature = Arc::new(
                        notist_core::signature::ResolvedSignature::from_shared(owned.clone())
                            .map_err(str::to_owned)?,
                    );
                    component.definition = owned;
                }
                Target::Component(component)
            }
            target => target,
        };
        // Native-only pages share the validated table. A page's first custom
        // binding detaches it without changing any other page or the defaults.
        self.identities.insert(definition.id.clone());
        Arc::make_mut(&mut self.targets).insert(definition.id.def_id(), target);
        Ok(())
    }
}
/// Stable external presentation order, independent of numeric allocation order.
pub fn component_definitions<'a>(
    root: &Item,
    registry: &'a notist_core::registry::Registry,
) -> Vec<&'a FunctionDef> {
    let mut definitions: Vec<_> = component_ids(root)
        .into_iter()
        .filter_map(|id| registry.definition(id))
        .collect();
    definitions.sort_by(|a, b| a.id.cmp(&b.id));
    definitions
}

// Lowercase ASCII names stay readable. Escapes are
// deterministic; registration rejects every collision, including separators.
pub fn encode_name(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
            out.push(c);
        } else {
            out.push_str(&format!("u{:x}x", c as u32));
        }
    }
    if !out.starts_with(|c: char| c.is_ascii_lowercase()) {
        out.insert(0, 'x');
    }
    out
}
/// Stable resource key independent of any import alias.
pub fn module_key(module: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in module.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}
pub fn component_tag(id: &FunctionId) -> String {
    format!(
        "notist-{}-{}",
        module_key(&id.module),
        encode_name(&id.name)
    )
}
pub fn parameter_attribute(name: &str) -> String {
    format!("notist-{}", encode_name(name))
}

/// Scalar fields remain readable attributes; collections use typed JSON v1.
pub fn encode_parameter(value: &Value) -> Result<String, &'static str> {
    if value.contains_data_views() {
        return Err(
            "Data and DataError cannot be exported as HTML or component data; decode selected ordinary values first",
        );
    }
    if value.contains_path() {
        return Err(
            "Path cannot be exported as HTML or component data; publish the resource explicitly",
        );
    }
    Ok(match value {
        Value::Str(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) if value.is_nan() => "NaN".into(),
        Value::Float(value) if *value == f64::INFINITY => "Infinity".into(),
        Value::Float(value) if *value == f64::NEG_INFINITY => "-Infinity".into(),
        Value::Float(value) => value.to_string(),
        Value::Unit => "null".into(),
        Value::Path(_) => unreachable!("paths were rejected above"),
        Value::Data(_) | Value::DataError(_) => unreachable!("data views were rejected above"),
        Value::Array(_)
        | Value::Dict(_)
        | Value::Some(_)
        | Value::None
        | Value::Ok(_)
        | Value::Err(_) => value_json(value).to_string(),
    })
}
/// Typed Value wire representation shared with IR inspection tools.
pub fn value_json(value: &Value) -> serde_json::Value {
    use serde_json::json;
    match value {
        Value::Unit => json!(["unit"]),
        Value::Bool(v) => json!(["bool", v]),
        Value::Int(v) => json!(["int", v.to_string()]),
        Value::Float(v) => json!(["float", format!("{:016x}", v.to_bits())]),
        Value::Str(v) => json!(["string", v.as_str()]),
        Value::Path(v) => json!(["path", {"root":v.root(), "path":v.path()}]),
        Value::Data(v) => {
            json!(["data-summary", {"kind":v.kind(), "nodes":v.node_count(), "bytes":v.logical_size()}])
        }
        Value::DataError(v) => json!(["data-error-summary", {"kind":v.kind().as_str()}]),
        Value::Array(v) => json!(["array", v.iter().map(value_json).collect::<Vec<_>>()]),
        Value::Dict(v) => json!([
            "dict",
            v.iter()
                .map(|(key, value)| json!([key, value_json(value)]))
                .collect::<Vec<_>>()
        ]),
        Value::Some(v) => json!(["some", value_json(v)]),
        Value::None => json!(["none"]),
        Value::Ok(v) => json!(["ok", value_json(v)]),
        Value::Err(v) => json!(["err", value_json(v)]),
    }
}
