//! Definitions, lexical module scopes and explicitly selected output mappings.
use crate::transforms::{Replace, TransformPlan};
use notist_core::{builtins, definitions::FunctionDef, diag::Diagnostic, registry::Registry};
use std::collections::BTreeMap;

#[derive(Debug, Clone)]
/// Immutable definitions, lexical module scopes and checked execution mappings.
/// Hosts load sources and resources; Context performs no IO.
pub struct Context {
    registry: Registry,
    transforms: TransformPlan,
    program: crate::definitions::CodeProgram,
    root: Option<String>,
}
impl Context {
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            transforms: Default::default(),
            program: Default::default(),
            root: None,
        }
    }
    /// Install a module and select it as the root lexical scope.
    pub fn with_module(
        mut self,
        module: crate::definitions::CodeModule,
    ) -> Result<Self, Vec<Diagnostic>> {
        self.root = Some(module.definitions.module.clone());
        self.registry.register(module.definitions)?;
        self.program.bodies.extend(module.program.bodies);
        self.program.sources.extend(module.program.sources);
        self.program.modules.extend(module.program.modules);
        self.program.imports.extend(module.program.imports);
        Ok(self)
    }
    pub fn with_root(mut self, module: impl Into<String>) -> Self {
        self.root = Some(module.into());
        self
    }
    /// Install a file's static import bindings and select its lexical scope.
    pub fn with_scope(
        mut self,
        module: impl Into<String>,
        imports: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let module = module.into();
        self.program
            .imports
            .insert(module.clone(), imports.into_iter().collect());
        self.root = Some(module);
        self
    }
    pub fn root(&self) -> Option<&str> {
        self.root.as_deref()
    }
    /// Resolve a name in one lexical module. Loaded modules are never ambient
    /// globals; imports are exported namespace bindings and can be traversed.
    pub fn resolve_in(&self, scope: Option<&str>, path: &str) -> Option<&FunctionDef> {
        self.resolve_from(scope.or(self.root()), None, path)
    }
    /// Resolve in a temporary import table, without inheriting the root scope.
    /// Module definitions and member traversal use the same resolver as Code.
    pub fn resolve_imports(
        &self,
        imports: &BTreeMap<String, String>,
        path: &str,
    ) -> Option<&FunctionDef> {
        self.resolve_from(None, Some(imports), path)
    }
    fn resolve_from(
        &self,
        scope: Option<&str>,
        imports: Option<&BTreeMap<String, String>>,
        path: &str,
    ) -> Option<&FunctionDef> {
        let mut parts: Vec<_> = path.split('.').collect();
        let name = parts.pop()?;
        let mut module = scope;
        for (i, part) in parts.iter().enumerate() {
            if i == 0 && *part == "notist" {
                module = Some("notist");
                continue;
            }
            if i == 0 && *part == "self" {
                continue;
            }
            let bindings = if i == 0 && imports.is_some() {
                imports?
            } else {
                self.program.imports.get(module?)?
            };
            module = Some(bindings.get(*part)?);
        }
        if let Some(def) = module.and_then(|id| self.registry.function(id, name)) {
            return Some(def);
        }
        if path == name {
            self.registry.function("notist", name)
        } else {
            None
        }
    }
    pub fn resolve(&self, path: &str) -> Option<&FunctionDef> {
        self.resolve_in(self.root(), path)
    }
    pub fn program(&self) -> &crate::definitions::CodeProgram {
        &self.program
    }
    pub fn registry(&self) -> &Registry {
        &self.registry
    }
    pub fn transforms(&self) -> &TransformPlan {
        &self.transforms
    }
    pub fn with_transforms(mut self, rules: &[Replace]) -> Result<Self, Vec<Diagnostic>> {
        self.transforms = TransformPlan::compile(rules, &self.registry)?;
        Ok(self)
    }
}
impl Default for Context {
    fn default() -> Self {
        Self::new(builtins::registry().clone())
    }
}
