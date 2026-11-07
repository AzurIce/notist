//! Definitions, lexical module scopes and explicitly selected output mappings.
use crate::extensions::ExtensionPlan;
use crate::{
    BindingSpec, CodeModule, ExportTable, LinkedModule, ModuleExports, ModuleScope, bind_imports,
};
use notist_core::extensions::DocumentExtension;
use notist_core::{
    builtins,
    definitions::{DefinitionModule, FunctionDef, Import, ImportBinding},
    diag::{Diagnostic, Phase},
    registry::Registry,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone)]
/// Immutable definitions, lexical module scopes and checked execution mappings.
/// Hosts load sources and resources; Context performs no IO.
pub struct Context {
    registry: Registry,
    extensions: ExtensionPlan,
    program: crate::definitions::CodeProgram,
    root: Option<String>,
    resource_root: String,
    checked: Arc<Mutex<BTreeMap<String, crate::code::CheckedProgram>>>,
}
impl Context {
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            extensions: Default::default(),
            program: Default::default(),
            root: None,
            resource_root: "/".into(),
            checked: Default::default(),
        }
    }
    /// Install a file without imports and select its lexical scope.
    /// Files with imports must be linked against host-resolved targets first.
    pub fn with_module(self, module: impl Into<Arc<CodeModule>>) -> Result<Self, Vec<Diagnostic>> {
        let linked = self.link_module(module, std::iter::empty())?;
        self.with_linked_module(linked)
    }
    /// Resolve a compiled file's imports against this loaded dependency view.
    /// `targets` has one canonical module identity per source import, in order.
    pub fn link_module(
        &self,
        module: impl Into<Arc<CodeModule>>,
        targets: impl IntoIterator<Item = String>,
    ) -> Result<Arc<LinkedModule>, Vec<Diagnostic>> {
        #[cfg(feature = "profiling")]
        let _timer = crate::profiling::enter(crate::profiling::Stage::Link);
        let code = module.into();
        let targets: Vec<_> = targets.into_iter().collect();
        if code.imports.len() != targets.len() {
            return Err(vec![Diagnostic::new(
                Phase::Type,
                code.definitions.span,
                "module imports require one resolved target per declaration",
            )]);
        }
        let errors: Vec<_> = code
            .imports
            .iter()
            .zip(&targets)
            .filter_map(|(import, target)| {
                self.registry.module(target).is_none().then(|| {
                    Diagnostic::new(
                        Phase::Type,
                        import.span,
                        format!("import target `{target}` is not loaded"),
                    )
                })
            })
            .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
        let spec = BindingSpec::new(code.imports.iter().zip(targets), Some(&code.definitions));
        let scope = bind_imports(&spec, self)?;
        Ok(Arc::new(LinkedModule { code, scope }))
    }
    /// Install a linked file produced for this dependency view. The caller must
    /// not mix a scope from another dependency snapshot into this Context.
    pub fn with_linked_module(
        mut self,
        module: Arc<LinkedModule>,
    ) -> Result<Self, Vec<Diagnostic>> {
        #[cfg(feature = "profiling")]
        let _timer = crate::profiling::enter(crate::profiling::Stage::Install);
        self.registry.register(module.code().definitions.clone())?;
        self.checked = Default::default();
        self.root = Some(module.code().id().to_owned());
        self.program.install(module);
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
        imports: impl IntoIterator<Item = (String, ImportBinding)>,
    ) -> Self {
        let module = module.into();
        if self.program.module(&module).is_some() {
            // Overriding a Code module's lexical imports changes body meaning.
            self.checked = Default::default();
        }
        self.program.with_scope(
            module.clone(),
            Arc::new(ModuleScope::from_bindings(imports.into_iter().collect())),
        );
        self.root = Some(module);
        self
    }
    pub fn root(&self) -> Option<&str> {
        self.root.as_deref()
    }
    /// Select the logical root for document paths. The standalone pipeline uses
    /// `/`, a virtual root, never the process working directory. No IO occurs.
    pub fn with_resource_root(mut self, root: impl Into<String>) -> Self {
        self.resource_root = root.into();
        self
    }
    pub fn resource_root(&self) -> &str {
        &self.resource_root
    }
    pub(crate) fn checked_program(&self, key: &str) -> Option<crate::code::CheckedProgram> {
        self.checked.lock().unwrap().get(key).cloned()
    }
    pub(crate) fn remember_checked_program(
        &self,
        key: String,
        program: &crate::code::CheckedProgram,
    ) {
        let mut checked = self.checked.lock().unwrap();
        let records: usize = checked.values().map(|p| p.instances.len()).sum();
        if checked.len() < 32 && records.saturating_add(program.instances.len()) <= 8192 {
            checked.entry(key).or_insert_with(|| program.clone());
        }
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
        imports: &BTreeMap<String, ImportBinding>,
        path: &str,
    ) -> Option<&FunctionDef> {
        self.resolve_from(None, Some(imports), path)
    }
    fn resolve_from(
        &self,
        scope: Option<&str>,
        imports: Option<&BTreeMap<String, ImportBinding>>,
        path: &str,
    ) -> Option<&FunctionDef> {
        let mut parts = path.split('.').peekable();
        let mut module = scope.map(str::to_owned);
        let mut temporary = imports;
        let mut first = true;
        while let Some(part) = parts.next() {
            if first && matches!(part, "notist" | "self") && parts.peek().is_some() {
                if part == "notist" {
                    module = Some("notist".into());
                }
                temporary = None;
                first = false;
                continue;
            }
            let binding = if let Some(table) = temporary {
                table.get(part).cloned()
            } else {
                module
                    .as_deref()
                    .and_then(|module| self.public_binding(module, part))
            };
            match binding {
                Some(ImportBinding::Function(id)) if parts.peek().is_none() => {
                    return self.registry.get(&id);
                }
                Some(ImportBinding::Module(id)) if parts.peek().is_some() => {
                    module = Some(id);
                    temporary = None;
                }
                None if first && parts.peek().is_none() => {
                    return self.registry.function("notist", part);
                }
                _ => return None,
            }
            first = false;
        }
        None
    }

    fn public_binding(&self, module: &str, name: &str) -> Option<ImportBinding> {
        self.registry
            .function(module, name)
            .map(|f| ImportBinding::Function(f.id.clone()))
            .or_else(|| self.program.scope(module)?.imports().get(name).cloned())
    }

    /// Own definitions and static imports are public; the implicit prelude is not.
    pub fn public_bindings(&self, module: &str) -> BTreeMap<String, ImportBinding> {
        let mut bindings = self
            .program
            .scope(module)
            .map(|scope| scope.imports().clone())
            .unwrap_or_default();
        bindings.extend(
            self.registry
                .functions()
                .filter(|f| f.id.module == module)
                .map(|f| (f.id.name.clone(), ImportBinding::Function(f.id.clone()))),
        );
        bindings
    }

    /// Expand imports after their targets have loaded, checking one shared name
    /// table for namespaces, explicit members and wildcard members alike.
    pub fn import_bindings<'a>(
        &self,
        imports: impl IntoIterator<Item = (&'a Import, String)>,
        local: Option<&DefinitionModule>,
    ) -> Result<BTreeMap<String, ImportBinding>, Vec<Diagnostic>> {
        bind_imports(&BindingSpec::new(imports, local), self).map(|scope| scope.imports().clone())
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
    pub fn extensions(&self) -> &ExtensionPlan {
        &self.extensions
    }
    /// Register an extension after installing its module dependencies.
    pub fn with_extension(
        self,
        extension: impl DocumentExtension + 'static,
    ) -> Result<Self, Vec<Diagnostic>> {
        self.with_shared_extension(Arc::new(extension))
    }
    pub fn with_shared_extension(
        mut self,
        extension: Arc<dyn DocumentExtension>,
    ) -> Result<Self, Vec<Diagnostic>> {
        self.extensions = self.extensions.with_extension(extension, &self.registry)?;
        Ok(self)
    }
}
impl ModuleExports for Context {
    fn lookup(&self, module: &str, name: &str) -> Option<ImportBinding> {
        self.public_binding(module, name)
    }
    fn all(&self, module: &str) -> Arc<ExportTable> {
        // Source modules already own their public table. Native modules and
        // explicit scope overlays retain the same registry-first semantics.
        if let Some(linked) = self.program.module(module)
            && self
                .program
                .scope(module)
                .is_some_and(|scope| Arc::ptr_eq(scope, linked.scope()))
        {
            return linked.scope().exports().clone();
        }
        Arc::new(self.public_bindings(module))
    }
}
impl Default for Context {
    fn default() -> Self {
        Self::new(builtins::registry().clone())
    }
}

impl notist_eval::EvaluationContext for Context {
    fn has_mappings(&self) -> bool {
        self.extensions().has_replacements()
    }
    fn registry(&self) -> &Registry {
        self.registry()
    }

    fn resource_root(&self) -> &str {
        self.resource_root()
    }

    fn target(
        &self,
        id: &notist_core::definitions::FunctionId,
    ) -> notist_core::definitions::FunctionId {
        self.extensions().target(id)
    }

    fn specialize(
        &self,
        instance: &notist_core::definitions::FunctionInstance,
        program: crate::code::CheckedProgram,
    ) -> (crate::code::CheckedProgram, Vec<Diagnostic>) {
        let mut checker = crate::code::Checker::with_program(self, program);
        checker.install(instance);
        (checker.program, checker.diagnostics)
    }
}
