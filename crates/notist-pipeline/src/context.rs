//! Definitions, lexical module scopes and explicitly selected output mappings.
use crate::extensions::ExtensionPlan;
use notist_core::extensions::DocumentExtension;
use notist_core::{
    builtins,
    definitions::{DefinitionModule, FunctionDef, Import, ImportBinding, ImportKind},
    diag::{Diagnostic, Phase},
    registry::Registry,
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Debug, Clone)]
/// Immutable definitions, lexical module scopes and checked execution mappings.
/// Hosts load sources and resources; Context performs no IO.
pub struct Context {
    registry: Registry,
    extensions: ExtensionPlan,
    program: crate::definitions::CodeProgram,
    root: Option<String>,
}
impl Context {
    pub fn new(registry: Registry) -> Self {
        Self {
            registry,
            extensions: Default::default(),
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
        imports: impl IntoIterator<Item = (String, ImportBinding)>,
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
            .or_else(|| self.program.imports.get(module)?.get(name).cloned())
    }

    /// Own definitions and static imports are public; the implicit prelude is not.
    pub fn public_bindings(&self, module: &str) -> BTreeMap<String, ImportBinding> {
        let mut bindings = self
            .program
            .imports
            .get(module)
            .cloned()
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
        let mut bindings = BTreeMap::new();
        let mut origins = BTreeMap::new();
        let mut diagnostics = Vec::new();
        if let Some(local) = local {
            for f in &local.functions {
                origins.insert(
                    f.id.name.clone(),
                    format!("local function `{}` at {:?}", f.id, f.span),
                );
            }
        }
        for (import, module) in imports {
            let public = self.public_bindings(&module);
            let selected = match &import.kind {
                ImportKind::Module { alias } => vec![(
                    alias.clone(),
                    ImportBinding::Module(module.clone()),
                    import.span,
                    "module".to_owned(),
                )],
                ImportKind::Wildcard => public
                    .into_iter()
                    .map(|(name, binding)| {
                        (
                            name.clone(),
                            binding,
                            import.span,
                            format!("member `{name}`"),
                        )
                    })
                    .collect(),
                ImportKind::Members(members) => members
                    .iter()
                    .filter_map(|member| match public.get(&member.name) {
                        Some(binding) => Some((
                            member.alias.clone(),
                            binding.clone(),
                            member.span,
                            format!("member `{}`", member.name),
                        )),
                        None => {
                            diagnostics.push(Diagnostic::new(
                                Phase::Type,
                                member.span,
                                format!(
                                    "module `{}` ({module}) has no public member `{}`",
                                    import.path, member.name
                                ),
                            ));
                            None
                        }
                    })
                    .collect(),
            };
            for (name, binding, span, selected) in selected {
                let origin = format!("{selected} from `{}` ({module}) at {span:?}", import.path);
                if !crate::lowering::imports::valid_import_alias(&name) {
                    diagnostics.push(Diagnostic::new(
                        Phase::Type,
                        span,
                        format!("invalid or reserved import name `{name}`: {origin}"),
                    ));
                } else if let Some(previous) = origins.get(&name) {
                    diagnostics.push(Diagnostic::new(Phase::Type, span,
                        format!("import name conflict for `{name}`: {origin}; already bound by {previous}")));
                } else {
                    origins.insert(name.clone(), origin);
                    bindings.insert(name, binding);
                }
            }
        }
        if diagnostics.is_empty() {
            Ok(bindings)
        } else {
            Err(diagnostics)
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
impl Default for Context {
    fn default() -> Self {
        Self::new(builtins::registry().clone())
    }
}
