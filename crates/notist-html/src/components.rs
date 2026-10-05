//! Signature-bound HTML targets and the versioned component parameter protocol.
use notist_core::{
    builtins,
    definitions::{FunctionDef, FunctionId},
    item::{Ctor, Value},
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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

/// Relative candidates, in the package's logical resource root. Neither has priority.
pub fn component_entries(name: &str) -> [PathBuf; 2] {
    [
        PathBuf::from("components").join(format!("{name}.js")),
        PathBuf::from("components").join(name).join("index.js"),
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
    pub id: FunctionId,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct Component {
    pub id: FunctionId,
    pub tag: String,
    /// Browser module locator interpreted by the host, not the renderer.
    pub module: ModuleLocator,
    pub definition: FunctionDef,
}
#[derive(Debug, Clone)]
pub enum Target {
    Native(Ctor),
    Component(Component),
}

#[derive(Debug, Clone)]
pub struct HtmlRegistry {
    targets: BTreeMap<FunctionId, Target>,
}
impl Default for HtmlRegistry {
    fn default() -> Self {
        let mut registry = Self {
            targets: BTreeMap::new(),
        };
        for definition in builtins::registry().functions() {
            registry
                .register(
                    definition,
                    Target::Native(
                        Ctor::from_name(&definition.id.name).expect("native constructor"),
                    ),
                )
                .expect("valid native HTML target");
        }
        registry
    }
}
impl HtmlRegistry {
    /// Bind all targets using the authoritative semantic registry. Errors do
    /// not return a partially installed HTML environment; declarations remain valid.
    pub fn from_bindings(
        registry: &notist_core::registry::Registry,
        bindings: &BTreeMap<FunctionId, ModuleLocator>,
    ) -> Result<Self, Vec<BindingError>> {
        let mut html = Self::default();
        let mut errors = Vec::new();
        for (id, module) in bindings {
            let result = registry
                .get(id)
                .ok_or_else(|| format!("unknown component function `{id}`"))
                .and_then(|definition| html.bind_component(definition, module.clone()));
            if let Err(message) = result {
                errors.push(BindingError {
                    id: id.clone(),
                    message,
                });
            }
        }
        if errors.is_empty() {
            Ok(html)
        } else {
            Err(errors)
        }
    }
    pub fn get(&self, id: &FunctionId) -> Option<&Target> {
        self.targets.get(id)
    }
    pub fn bind_component(
        &mut self,
        definition: &FunctionDef,
        module: impl Into<ModuleLocator>,
    ) -> Result<(), String> {
        self.register(
            definition,
            Target::Component(Component {
                id: definition.id.clone(),
                tag: component_tag(&definition.id),
                module: module.into(),
                definition: definition.clone(),
            }),
        )
    }
    /// Both native handlers and components enter the same identity registry.
    pub fn register(&mut self, definition: &FunctionDef, target: Target) -> Result<(), String> {
        let mut module = notist_core::definitions::DefinitionModule::new(&definition.id.package);
        module.functions.push(definition.clone());
        let errors = notist_core::definitions::validate_module(&module);
        if !errors.is_empty() {
            return Err(errors
                .iter()
                .map(|error| error.message.as_str())
                .collect::<Vec<_>>()
                .join("; "));
        }
        if self.targets.contains_key(&definition.id) {
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
            let mut attributes = BTreeSet::from(["notist-protocol".to_owned()]);
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
                component.definition = definition.clone();
                Target::Component(component)
            }
            target => target,
        };
        self.targets.insert(definition.id.clone(), target);
        Ok(())
    }
}

// Ordinary ASCII names retain the package-function convention. Escapes are
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
pub fn component_tag(id: &FunctionId) -> String {
    format!("{}-{}", encode_name(&id.package), encode_name(&id.name))
}
pub fn parameter_attribute(name: &str) -> String {
    format!("notist-{}", encode_name(name))
}

/// Scalar fields remain readable attributes; collections use typed JSON v1.
pub fn encode_parameter(value: &Value) -> String {
    match value {
        Value::Str(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) if value.is_nan() => "NaN".into(),
        Value::Float(value) if *value == f64::INFINITY => "Infinity".into(),
        Value::Float(value) if *value == f64::NEG_INFINITY => "-Infinity".into(),
        Value::Float(value) => value.to_string(),
        Value::Unit => "null".into(),
        Value::Array(_) | Value::Dict(_) => value_json(value).to_string(),
    }
}
/// Typed Value wire representation shared with IR inspection tools.
pub fn value_json(value: &Value) -> serde_json::Value {
    use serde_json::json;
    match value {
        Value::Unit => json!(["unit"]),
        Value::Bool(v) => json!(["bool", v]),
        Value::Int(v) => json!(["int", v.to_string()]),
        Value::Float(v) => json!(["float", format!("{:016x}", v.to_bits())]),
        Value::Str(v) => json!(["string", v]),
        Value::Array(v) => json!(["array", v.iter().map(value_json).collect::<Vec<_>>()]),
        Value::Dict(v) => json!([
            "dict",
            v.iter()
                .map(|(key, value)| json!([key, value_json(value)]))
                .collect::<Vec<_>>()
        ]),
    }
}
