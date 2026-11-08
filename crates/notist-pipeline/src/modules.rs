//! File-local import binding, independent of source loading and function bodies.
use crate::CodeModule;
use comemo::{Track, Tracked};
use notist_core::{
    definitions::{
        DefinitionModule, FunctionId, Import, ImportBinding, ImportKind, ModuleId, ModuleRef,
    },
    diag::{Diagnostic, Phase},
};
use rowan::TextRange;
use std::{
    collections::BTreeMap,
    sync::{Arc, LazyLock},
};

pub type ExportTable = BTreeMap<String, ImportBinding>;

/// The narrow dependency surface needed by import binding. Queries perform no IO.
#[comemo::track]
pub trait ModuleExports {
    fn lookup(&self, module: ModuleId, name: &str) -> Option<ImportBinding>;
    fn all(&self, module: ModuleId) -> Arc<ExportTable>;
    fn function_identity(&self, id: notist_core::definitions::DefId) -> Option<FunctionId>;
    fn module_identity(&self, id: ModuleId) -> Option<ModuleRef>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LocalName {
    id: FunctionId,
    span: TextRange,
}

/// Only names, import selections, resolved targets and diagnostic positions.
/// Changing a function body does not change this specification unless positions move.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct BindingSpec {
    locals: Vec<LocalName>,
    imports: Vec<(Import, ModuleRef)>,
}

impl BindingSpec {
    /// Targets must already have been resolved and loaded by the host.
    pub fn new<'a>(
        imports: impl IntoIterator<Item = (&'a Import, String)>,
        local: Option<&DefinitionModule>,
    ) -> Self {
        Self::resolved(
            imports
                .into_iter()
                .map(|(import, name)| (import, ModuleRef::new(name))),
            local,
        )
    }
    pub fn resolved<'a>(
        imports: impl IntoIterator<Item = (&'a Import, ModuleRef)>,
        local: Option<&DefinitionModule>,
    ) -> Self {
        Self {
            locals: local
                .into_iter()
                .flat_map(|m| &m.functions)
                .map(|f| LocalName {
                    id: f.id.clone(),
                    span: f.span,
                })
                .collect(),
            imports: imports
                .into_iter()
                .map(|(import, target)| (import.clone(), target))
                .collect(),
        }
    }
}

/// Resolved names only: no signatures, function bodies, or dependency handles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModuleScope {
    identities: notist_core::definition_identity::Definitions,
    imports: ExportTable,
    exports: Arc<ExportTable>,
}

impl ModuleScope {
    pub fn imports(&self) -> &ExportTable {
        &self.imports
    }
    pub fn exports(&self) -> &Arc<ExportTable> {
        &self.exports
    }
    pub fn identities(&self) -> &notist_core::definition_identity::Definitions {
        &self.identities
    }
    pub(crate) fn from_bindings(
        imports: ExportTable,
        identities: notist_core::definition_identity::Definitions,
    ) -> Self {
        Self {
            identities,
            exports: Arc::new(imports.clone()),
            imports,
        }
    }
}

/// A file's shared compilation output and its separately resolved lexical scope.
/// Dependency identities are interpreted in the Context this module was linked for.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedModule {
    pub(crate) code: Arc<CodeModule>,
    pub(crate) scope: Arc<ModuleScope>,
}

impl LinkedModule {
    pub fn code(&self) -> &Arc<CodeModule> {
        &self.code
    }
    pub fn scope(&self) -> &Arc<ModuleScope> {
        &self.scope
    }
}

/// Expand imports in one shared name table. The host owns graph validity;
/// this operation only observes public names and never changes the environment.
pub fn bind_imports(
    spec: &BindingSpec,
    exports: &dyn ModuleExports,
) -> Result<Arc<ModuleScope>, Vec<Diagnostic>> {
    #[cfg(feature = "profiling")]
    let _timer = crate::profiling::enter(crate::profiling::Stage::Bind);
    if cache_enabled() && spec.locals.is_empty() && spec.imports.is_empty() {
        // Ordinary documents without imports have nothing to hash or validate.
        static EMPTY: LazyLock<Arc<ModuleScope>> =
            LazyLock::new(|| Arc::new(ModuleScope::default()));
        return Ok(EMPTY.clone());
    }
    bind_cached(spec, exports.track())
}

fn cache_enabled() -> bool {
    #[cfg(feature = "profiling")]
    {
        crate::profiling::binding_enabled()
    }
    #[cfg(not(feature = "profiling"))]
    {
        true
    }
}

#[comemo::memoize(enabled = cache_enabled())]
fn bind_cached(
    spec: &BindingSpec,
    exports: Tracked<dyn ModuleExports + '_>,
) -> Result<Arc<ModuleScope>, Vec<Diagnostic>> {
    #[cfg(feature = "profiling")]
    let _timer = crate::profiling::enter(crate::profiling::Stage::BindWork);
    let mut bindings = BTreeMap::new();
    let mut origins = BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut identities = notist_core::definition_identity::Definitions::default();
    for local in &spec.locals {
        identities.insert(local.id.clone());
        origins.insert(
            local.id.name.clone(),
            format!("local function `{}` at {:?}", local.id, local.span),
        );
    }
    for (import, module) in &spec.imports {
        identities.insert_module(module.clone());
        let selected = match &import.kind {
            ImportKind::Module { alias } => vec![(
                alias.clone(),
                ImportBinding::Module(module.id()),
                import.span,
                "module".to_owned(),
            )],
            ImportKind::Wildcard => exports
                .all(module.id())
                .iter()
                .map(|(name, binding)| {
                    (
                        name.clone(),
                        binding.clone(),
                        import.span,
                        format!("member `{name}`"),
                    )
                })
                .collect(),
            ImportKind::Members(members) => members
                .iter()
                .filter_map(|member| match exports.lookup(module.id(), &member.name) {
                    Some(binding) => Some((
                        member.alias.clone(),
                        binding,
                        member.span,
                        format!("member `{}`", member.name),
                    )),
                    None => {
                        diagnostics.push(Diagnostic::new(
                            Phase::Type,
                            member.span,
                            format!(
                                "module `{}` ({module}) has no public member `{}`",
                                import.path,
                                member.name,
                                module = module.name()
                            ),
                        ));
                        None
                    }
                })
                .collect(),
        };
        for (name, binding, span, selected) in selected {
            let origin = format!(
                "{selected} from `{}` ({}) at {span:?}",
                import.path,
                module.name()
            );
            if !crate::lowering::imports::valid_import_alias(&name) {
                diagnostics.push(Diagnostic::new(
                    Phase::Type,
                    span,
                    format!("invalid or reserved import name `{name}`: {origin}"),
                ));
            } else if let Some(previous) = origins.get(&name) {
                diagnostics.push(Diagnostic::new(
                    Phase::Type,
                    span,
                    format!(
                        "import name conflict for `{name}`: {origin}; already bound by {previous}"
                    ),
                ));
            } else {
                origins.insert(name.clone(), origin);
                bindings.insert(name, binding);
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    for binding in bindings.values() {
        let owned = match binding {
            ImportBinding::Function(id) => exports.function_identity(*id).map(|owner| {
                identities.insert(owner);
            }),
            ImportBinding::Module(id) if identities.module(*id).is_ok() => Some(()),
            ImportBinding::Module(id) => exports.module_identity(*id).map(|owner| {
                identities.insert_module(owner);
            }),
        };
        if owned.is_none() {
            return Err(vec![Diagnostic::new(
                Phase::Type,
                Default::default(),
                "resolved import has no retained definition identity",
            )]);
        }
    }
    let mut public = bindings.clone();
    public.extend(spec.locals.iter().map(|local| {
        (
            local.id.name.clone(),
            ImportBinding::Function(local.id.def_id()),
        )
    }));
    Ok(Arc::new(ModuleScope {
        identities,
        imports: bindings,
        exports: Arc::new(public),
    }))
}
