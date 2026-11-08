//! Definitions indexed by canonical module identity, independent of import aliases.
use crate::definitions::{
    DefId, DefinitionModule, FunctionDef, FunctionId, ModuleId, validate_module,
};
use crate::diag::{Diagnostic, Phase};
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Debug)]
struct ModuleFunctions {
    definitions: Arc<DefinitionModule>,
    signatures: Vec<Arc<crate::signature::ResolvedSignature>>,
    names: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Default)]
pub struct Registry {
    identities: crate::definition_identity::Definitions,
    // Cloning a Context shares definitions. Installing a module detaches only
    // the module index; existing modules and their checked signatures stay shared.
    modules: Arc<BTreeMap<String, ModuleId>>,
    // Registry-local snapshots, never process-global implementation caches.
    definitions: Arc<BTreeMap<DefId, (Arc<ModuleFunctions>, usize)>>,
    module_ids: Arc<BTreeMap<ModuleId, Arc<ModuleFunctions>>>,
}
impl Registry {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn register(
        &mut self,
        module: impl Into<Arc<DefinitionModule>>,
    ) -> Result<(), Vec<Diagnostic>> {
        self.install_shared(module.into(), false)
    }
    pub(crate) fn install(
        &mut self,
        module: DefinitionModule,
        builtin: bool,
    ) -> Result<(), Vec<Diagnostic>> {
        self.install_shared(Arc::new(module), builtin)
    }
    fn install_shared(
        &mut self,
        module: Arc<DefinitionModule>,
        builtin: bool,
    ) -> Result<(), Vec<Diagnostic>> {
        let mut errors = validate_module(&module);
        if (module.module == "notist" || module.module.starts_with("notist.")) && !builtin {
            errors.push(Diagnostic::new(
                Phase::Type,
                module.span,
                "module identity `notist` is reserved",
            ));
        }
        if self.modules.contains_key(module.module.name()) {
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
        let functions = Arc::new(ModuleFunctions {
            signatures: module
                .functions
                .iter()
                .enumerate()
                .map(|(index, definition)| {
                    crate::signature::ResolvedSignature::from_module(module.clone(), index)
                        .map(Arc::new)
                        .map_err(|message| {
                            vec![Diagnostic::new(Phase::Type, definition.span, message)]
                        })
                })
                .collect::<Result<_, _>>()?,
            names: module
                .functions
                .iter()
                .enumerate()
                .map(|(i, f)| (f.id.name.clone(), i))
                .collect(),
            definitions: module,
        });
        for (index, definition) in functions.definitions.functions.iter().enumerate() {
            self.identities.insert(definition.id.clone());
            Arc::make_mut(&mut self.definitions)
                .insert(definition.id.def_id(), (functions.clone(), index));
        }
        Arc::make_mut(&mut self.module_ids)
            .insert(functions.definitions.module.id(), functions.clone());
        self.identities
            .insert_module(functions.definitions.module.clone());
        Arc::make_mut(&mut self.modules).insert(
            functions.definitions.module.name().to_owned(),
            functions.definitions.module.id(),
        );
        Ok(())
    }
    pub fn get(&self, id: &FunctionId) -> Option<&FunctionDef> {
        self.definition(id.def_id())
    }
    pub fn definition(&self, id: DefId) -> Option<&FunctionDef> {
        let (module, index) = self.definitions.get(&id)?;
        module.definitions.functions.get(*index)
    }
    pub fn signature(&self, id: DefId) -> Option<&Arc<crate::signature::ResolvedSignature>> {
        let (module, index) = self.definitions.get(&id)?;
        module.signatures.get(*index)
    }
    pub fn identities(&self) -> &crate::definition_identity::Definitions {
        &self.identities
    }
    pub fn function_in(&self, id: ModuleId, name: &str) -> Option<&FunctionDef> {
        let module = self.module_ids.get(&id)?;
        Some(&module.definitions.functions[*module.names.get(name)?])
    }
    pub fn module_by_id(&self, id: ModuleId) -> Option<&Arc<DefinitionModule>> {
        Some(&self.module_ids.get(&id)?.definitions)
    }
    pub fn function(&self, module: &str, name: &str) -> Option<&FunctionDef> {
        self.function_in(*self.modules.get(module)?, name)
    }
    pub fn module(&self, module: &str) -> Option<&Arc<DefinitionModule>> {
        self.module_by_id(*self.modules.get(module)?)
    }
    pub fn functions(&self) -> impl Iterator<Item = &FunctionDef> {
        self.modules
            .values()
            .filter_map(|id| self.module_ids.get(id))
            .flat_map(|m| m.names.values().map(|&i| &m.definitions.functions[i]))
    }
}
