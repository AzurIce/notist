//! Code tooling loads the edited module's own import graph from an overlay.
use super::*;
use crate::{DefinitionModule, Diagnostic, OverlayResources, Sources};

#[derive(Debug)]
pub struct CodeAnalysis {
    pub definitions: Option<DefinitionModule>,
    pub diagnostics: Vec<Diagnostic>,
    pub sources: Sources,
    environment: Option<Arc<Environment>>,
}
impl CodeAnalysis {
    /// The exact loaded import graph used by checking, reusable by tooling.
    pub fn environment(&self) -> Option<&Arc<Environment>> {
        self.environment.as_ref()
    }

    /// Source navigation from the same compiled module, without parsing again.
    pub fn navigation(&self) -> crate::SourceNavigation {
        let Some(environment) = &self.environment else {
            return Default::default();
        };
        let context = environment.context();
        context
            .root_id()
            .and_then(|id| context.program().module_by_id(id))
            .map(|module| crate::SourceNavigation::from_module(module.code(), context))
            .unwrap_or_default()
    }
}
impl<R: Resources> Vault<R> {
    pub fn analyze_code(
        &self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<CodeAnalysis, VaultError> {
        let path = self.resources.resolve(path.as_ref());
        let id = path.to_string_lossy().into_owned();
        let overlay = OverlayResources::new(
            &self.resources,
            &BTreeMap::from([(path.clone(), source.into())]),
        );
        match Environment::load_from(&overlay, &path) {
            Ok(environment) => {
                let context = environment.context();
                let module_id = context
                    .root_id()
                    .expect("loaded Code environment has a root module");
                let definitions = DefinitionModule {
                    module: context
                        .identities()
                        .module(module_id)
                        .expect("root module name is retained")
                        .clone(),
                    functions: context
                        .registry()
                        .functions()
                        .filter(|f| f.id.module_id() == module_id)
                        .cloned()
                        .collect(),
                    span: crate::TextRange::new(0.into(), (source.len() as u32).into()),
                };
                Ok(CodeAnalysis {
                    definitions: Some(definitions),
                    diagnostics: self.pipeline.check_module(&id, context),
                    sources: context.program().sources(),
                    environment: Some(Arc::new(environment)),
                })
            }
            Err(errors) => {
                let primary = crate::Source::in_namespace(
                    crate::SourceNamespace::named(self.resources.root().to_string_lossy().as_ref()),
                    id,
                    Some(source.into()),
                )?;
                let mut sources = Sources::from(primary);
                let mut diagnostics = Vec::new();
                for error in errors {
                    let owner = match error.diagnostic.source.as_ref() {
                        Some(source) if source.text().is_some() => source.clone(),
                        _ => crate::Source::in_namespace(
                            crate::SourceNamespace::named(
                                self.resources.root().to_string_lossy().as_ref(),
                            ),
                            error.path.to_string_lossy().as_ref(),
                            Some(error.source.into()),
                        )?,
                    };
                    sources.insert(owner.clone())?;
                    let mut diagnostic = error.diagnostic;
                    diagnostic.source.get_or_insert(owner);
                    diagnostics.push(diagnostic);
                }
                Ok(CodeAnalysis {
                    definitions: None,
                    diagnostics,
                    sources,
                    environment: None,
                })
            }
        }
    }
}
