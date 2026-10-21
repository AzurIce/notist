//! Definitions indexed by canonical module identity, independent of import aliases.
use crate::definitions::{DefinitionModule, FunctionDef, FunctionId, validate_module};
use crate::diag::{Diagnostic, Phase};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Registry {
    modules: BTreeMap<String, BTreeMap<String, FunctionDef>>,
}
impl Registry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(&mut self, module: DefinitionModule) -> Result<(), Vec<Diagnostic>> {
        self.install(module, false)
    }
    pub(crate) fn install(
        &mut self,
        module: DefinitionModule,
        builtin: bool,
    ) -> Result<(), Vec<Diagnostic>> {
        let mut errors = validate_module(&module);
        if module.module == "notist" && !builtin {
            errors.push(Diagnostic::new(
                Phase::Type,
                module.span,
                "module identity `notist` is reserved",
            ));
        }
        if self.modules.contains_key(&module.module) {
            errors.push(Diagnostic::new(
                Phase::Type,
                module.span,
                format!("module `{}` is already registered", module.module),
            ));
        }
        for function in &module.functions {
            let target = match function.children {
                crate::builtins::Accepts::Rows => Some("row"),
                crate::builtins::Accepts::Cells => Some("cell"),
                crate::builtins::Accepts::Items => Some("item"),
                _ => None,
            };
            if let Some(target) = target
                && self.function("notist", target).is_none()
                && !module
                    .functions
                    .iter()
                    .any(|f| f.id.module == "notist" && f.id.name == target)
            {
                errors.push(Diagnostic::new(
                    Phase::Type,
                    function.span,
                    format!("children contract references missing function `notist.{target}`"),
                ));
            }
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        self.modules.insert(
            module.module,
            module
                .functions
                .into_iter()
                .map(|f| (f.id.name.clone(), f))
                .collect(),
        );
        Ok(())
    }
    pub fn get(&self, id: &FunctionId) -> Option<&FunctionDef> {
        self.function(&id.module, &id.name)
    }
    pub fn function(&self, module: &str, name: &str) -> Option<&FunctionDef> {
        self.modules.get(module)?.get(name)
    }
    pub fn functions(&self) -> impl Iterator<Item = &FunctionDef> {
        self.modules.values().flat_map(|m| m.values())
    }
}
