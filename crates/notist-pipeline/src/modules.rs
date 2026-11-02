//! File-local import binding, independent of source loading and function bodies.
use crate::CodeModule;
use notist_core::{
    definitions::{DefinitionModule, FunctionId, Import, ImportBinding, ImportKind},
    diag::{Diagnostic, Phase},
};
use rowan::TextRange;
use std::{collections::BTreeMap, sync::Arc};

pub type ExportTable = BTreeMap<String, ImportBinding>;

/// The narrow dependency surface needed by import binding. Queries perform no IO.
pub trait ModuleExports {
    fn lookup(&self, module: &str, name: &str) -> Option<ImportBinding>;
    fn all(&self, module: &str) -> Arc<ExportTable>;
}

#[derive(Debug, Clone, PartialEq)]
struct LocalName {
    id: FunctionId,
    span: TextRange,
}

/// Only names, import selections, resolved targets and diagnostic positions.
/// Changing a function body does not change this specification unless positions move.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BindingSpec {
    locals: Vec<LocalName>,
    imports: Vec<(Import, String)>,
}

impl BindingSpec {
    /// Targets must already have been resolved and loaded by the host.
    pub fn new<'a>(
        imports: impl IntoIterator<Item = (&'a Import, String)>,
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
    pub(crate) fn from_bindings(imports: ExportTable) -> Self {
        Self {
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
    let mut bindings = BTreeMap::new();
    let mut origins = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for local in &spec.locals {
        origins.insert(
            local.id.name.clone(),
            format!("local function `{}` at {:?}", local.id, local.span),
        );
    }
    for (import, module) in &spec.imports {
        let selected = match &import.kind {
            ImportKind::Module { alias } => vec![(
                alias.clone(),
                ImportBinding::Module(module.clone()),
                import.span,
                "module".to_owned(),
            )],
            ImportKind::Wildcard => exports
                .all(module)
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
                .filter_map(|member| match exports.lookup(module, &member.name) {
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
    let mut public = bindings.clone();
    public.extend(spec.locals.iter().map(|local| {
        (
            local.id.name.clone(),
            ImportBinding::Function(local.id.clone()),
        )
    }));
    Ok(Arc::new(ModuleScope {
        imports: bindings,
        exports: Arc::new(public),
    }))
}
