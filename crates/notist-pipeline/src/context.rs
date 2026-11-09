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
    root: Option<notist_core::definitions::ModuleId>,
    identities: notist_core::definition_identity::Definitions,
    resource_root: String,
    checked: Arc<Mutex<BTreeMap<String, crate::code::CheckedProgram>>>,
}
impl Context {
    pub fn new(registry: Registry) -> Self {
        Self {
            identities: registry.identities().clone(),
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
        let timing = crate::progress::Span::stage("core.module.link", || "Link module".into());
        #[cfg(feature = "profiling")]
        let _timer = crate::profiling::enter(crate::profiling::Stage::Link);
        let code = CodeModule::linked_snapshot(module.into());
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
        timing.finish();
        Ok(Arc::new(LinkedModule { code, scope }))
    }
    pub fn link_module_resolved(
        &self,
        module: impl Into<Arc<CodeModule>>,
        targets: impl IntoIterator<Item = notist_core::definitions::ModuleId>,
    ) -> Result<Arc<LinkedModule>, Vec<Diagnostic>> {
        let timing =
            crate::progress::Span::stage("core.module.link", || "Link resolved module".into());
        #[cfg(feature = "profiling")]
        let _timer = crate::profiling::enter(crate::profiling::Stage::Link);
        let code = CodeModule::linked_snapshot(module.into());
        let targets: Vec<_> = targets.into_iter().collect();
        if targets.len() != code.imports.len() {
            return Err(vec![Diagnostic::new(
                Phase::Type,
                code.definitions.span,
                "module imports require one resolved target per declaration",
            )]);
        }
        let owners = code
            .imports
            .iter()
            .zip(targets)
            .map(|(import, id)| {
                self.identities
                    .module(id)
                    .ok()
                    .filter(|_| self.registry.module_by_id(id).is_some())
                    .cloned()
                    .map(|owner| (import, owner))
                    .ok_or_else(|| {
                        Diagnostic::new(
                            Phase::Type,
                            import.span,
                            "resolved import target is not loaded in this context",
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| vec![error])?;
        let scope = bind_imports(
            &BindingSpec::resolved(owners, Some(&code.definitions)),
            self,
        )?;
        timing.finish();
        Ok(Arc::new(LinkedModule { code, scope }))
    }
    /// Install a linked file produced for this dependency view. The caller must
    /// not mix a scope from another dependency snapshot into this Context.
    pub fn with_linked_module(
        mut self,
        module: Arc<LinkedModule>,
    ) -> Result<Self, Vec<Diagnostic>> {
        let timing = crate::progress::Span::stage("core.module.install", || {
            format!("Install module {}", module.code().id())
        });
        timing.attribute("module", || module.code().id().into());
        #[cfg(feature = "profiling")]
        let _timer = crate::profiling::enter(crate::profiling::Stage::Install);
        self.registry.register(module.code().definitions.clone())?;
        self.identities.merge(self.registry.identities());
        self.checked = Default::default();
        self.root = Some(module.code().module_id());
        self.program
            .install(module, &self.identities, self.registry.identities())
            .map_err(|error| {
                vec![Diagnostic::new(
                    Phase::Type,
                    Default::default(),
                    error.to_string(),
                )]
            })?;
        timing.finish();
        Ok(self)
    }
    pub fn with_root(mut self, module: impl Into<String>) -> Result<Self, Vec<Diagnostic>> {
        let module = module.into();
        let id = self.identities.module_id(&module).ok_or_else(|| {
            vec![Diagnostic::new(
                Phase::Type,
                Default::default(),
                format!("root module `{module}` is not loaded"),
            )]
        })?;
        self.root = Some(id);
        Ok(self)
    }
    /// Install a file's static import bindings and select its lexical scope.
    pub fn with_scope(
        mut self,
        module: impl Into<String>,
        imports: impl IntoIterator<Item = (String, ImportBinding)>,
    ) -> Result<Self, Vec<Diagnostic>> {
        let module = module.into();
        let imports: BTreeMap<_, _> = imports.into_iter().collect();
        let errors: Vec<_> = imports
            .iter()
            .filter_map(|(alias, binding)| {
                let owned = match binding {
                    ImportBinding::Function(id) => self.identities.function(*id).map(|_| ()),
                    ImportBinding::Module(id) => self.identities.module(*id).map(|_| ()),
                };
                owned.err().map(|error| {
                    Diagnostic::new(
                        Phase::Type,
                        Default::default(),
                        format!("invalid import binding `{alias}`: {error}"),
                    )
                })
            })
            .collect();
        if !errors.is_empty() {
            return Err(errors);
        }
        if self.program.module(&module).is_some() {
            // Overriding a Code module's lexical imports changes body meaning.
            self.checked = Default::default();
        }
        let owner = notist_core::definitions::ModuleRef::try_new(module).map_err(|error| {
            vec![Diagnostic::new(
                Phase::Type,
                Default::default(),
                error.to_string(),
            )]
        })?;
        let id = self.identities.insert_module(owner);
        self.program.with_scope(
            id,
            Arc::new(ModuleScope::from_bindings(imports, self.identities.clone())),
            &self.identities,
        );
        self.root = Some(id);
        Ok(self)
    }
    pub fn root(&self) -> Option<&str> {
        Some(self.identities.module(self.root?).ok()?.name())
    }
    pub fn root_id(&self) -> Option<notist_core::definitions::ModuleId> {
        self.root
    }
    pub fn identities(&self) -> &notist_core::definition_identity::Definitions {
        &self.identities
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
    pub fn resolve_in_id(
        &self,
        scope: Option<notist_core::definitions::ModuleId>,
        path: &str,
    ) -> Option<&FunctionDef> {
        self.resolve_from_id(scope.or(self.root), None, path)
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
        self.resolve_from_id(
            scope.and_then(|name| self.identities.module_id(name)),
            imports,
            path,
        )
    }
    fn resolve_from_id(
        &self,
        scope: Option<notist_core::definitions::ModuleId>,
        imports: Option<&BTreeMap<String, ImportBinding>>,
        path: &str,
    ) -> Option<&FunctionDef> {
        let mut parts = path.split('.').peekable();
        let mut module = scope;
        let mut temporary = imports;
        let mut first = true;
        while let Some(part) = parts.next() {
            if first && matches!(part, "notist" | "self") && parts.peek().is_some() {
                if part == "notist" {
                    module = self.identities.module_id("notist");
                }
                temporary = None;
                first = false;
                continue;
            }
            let binding = if let Some(table) = temporary {
                table.get(part).cloned()
            } else {
                module
                    .as_ref()
                    .and_then(|module| self.public_binding(*module, part))
            };
            match binding {
                Some(ImportBinding::Function(id)) if parts.peek().is_none() => {
                    return self.registry.definition(id);
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

    fn public_binding(
        &self,
        module: notist_core::definitions::ModuleId,
        name: &str,
    ) -> Option<ImportBinding> {
        self.registry
            .function_in(module, name)
            .map(|f| ImportBinding::Function(f.id.def_id()))
            .or_else(|| {
                self.program
                    .scope_by_id(module)?
                    .imports()
                    .get(name)
                    .cloned()
            })
    }

    /// Own definitions and static imports are public; the implicit prelude is not.
    pub fn public_bindings(&self, module: &str) -> BTreeMap<String, ImportBinding> {
        self.identities
            .module_id(module)
            .map(|id| self.public_bindings_id(id))
            .unwrap_or_default()
    }
    fn public_bindings_id(
        &self,
        module: notist_core::definitions::ModuleId,
    ) -> BTreeMap<String, ImportBinding> {
        let mut bindings = self
            .program
            .scope_by_id(module)
            .map(|scope| scope.imports().clone())
            .unwrap_or_default();
        bindings.extend(
            self.registry
                .functions()
                .filter(|f| f.id.module_id() == module)
                .map(|f| (f.id.name.clone(), ImportBinding::Function(f.id.def_id()))),
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
        let targets = imports
            .into_iter()
            .map(|(import, name)| {
                self.identities
                    .module_id(&name)
                    .map(|id| (import, id))
                    .ok_or_else(|| {
                        vec![Diagnostic::new(
                            Phase::Type,
                            import.span,
                            format!("import target `{name}` is not loaded"),
                        )]
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.import_bindings_resolved(targets, local)
    }
    pub fn import_bindings_resolved<'a>(
        &self,
        imports: impl IntoIterator<Item = (&'a Import, notist_core::definitions::ModuleId)>,
        local: Option<&DefinitionModule>,
    ) -> Result<BTreeMap<String, ImportBinding>, Vec<Diagnostic>> {
        let owners = imports
            .into_iter()
            .map(|(import, id)| {
                self.identities
                    .module(id)
                    .cloned()
                    .map(|owner| (import, owner))
                    .map_err(|error| {
                        vec![Diagnostic::new(Phase::Type, import.span, error.to_string())]
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        bind_imports(&BindingSpec::resolved(owners, local), self)
            .map(|scope| scope.imports().clone())
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
    fn function_identity(
        &self,
        id: notist_core::definitions::DefId,
    ) -> Option<notist_core::definitions::FunctionId> {
        self.identities.function(id).ok().cloned()
    }
    fn module_identity(
        &self,
        id: notist_core::definitions::ModuleId,
    ) -> Option<notist_core::definitions::ModuleRef> {
        self.identities.module(id).ok().cloned()
    }
    fn lookup(
        &self,
        module: notist_core::definitions::ModuleId,
        name: &str,
    ) -> Option<ImportBinding> {
        self.public_binding(module, name)
    }
    fn all(&self, module: notist_core::definitions::ModuleId) -> Arc<ExportTable> {
        // Source modules already own their public table. Native modules and
        // explicit scope overlays retain the same registry-first semantics.
        if let Some(linked) = self.program.module_by_id(module)
            && self
                .program
                .scope_by_id(module)
                .is_some_and(|scope| Arc::ptr_eq(scope, linked.scope()))
        {
            return linked.scope().exports().clone();
        }
        Arc::new(self.public_bindings_id(module))
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
        self.specialize_resolved(&instance.clone().into(), program)
    }
    fn target_resolved(
        &self,
        id: notist_core::definitions::DefId,
    ) -> notist_core::definitions::DefId {
        self.extensions().target_id(id)
    }
    fn specialize_resolved(
        &self,
        instance: &notist_core::definitions::ResolvedInstance,
        program: crate::code::CheckedProgram,
    ) -> (crate::code::CheckedProgram, Vec<Diagnostic>) {
        let mut checker = crate::code::Checker::with_program(self, program);
        checker.install(instance);
        (checker.program, checker.diagnostics)
    }
}
