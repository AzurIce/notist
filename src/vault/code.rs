//! Code tooling loads the edited module's own import graph from an overlay.
use super::*;
use crate::{DefinitionModule, Diagnostic, OverlayResources, Sources};

#[derive(Debug)]
pub struct CodeAnalysis {
    pub definitions: Option<DefinitionModule>,
    pub diagnostics: Vec<Diagnostic>,
    pub sources: Sources,
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
                let definitions = DefinitionModule {
                    module: id.clone(),
                    functions: context
                        .registry()
                        .functions()
                        .filter(|f| f.id.module == id)
                        .cloned()
                        .collect(),
                    span: crate::TextRange::new(0.into(), (source.len() as u32).into()),
                };
                Ok(CodeAnalysis {
                    definitions: Some(definitions),
                    diagnostics: self.pipeline.check_module(&id, context),
                    sources: context.program().sources().clone(),
                })
            }
            Err(errors) => {
                let mut sources = BTreeMap::from([(id, source.into())]);
                let diagnostics = errors
                    .into_iter()
                    .map(|error| {
                        let name = error.path.to_string_lossy().into_owned();
                        sources.insert(name.clone(), error.source.into());
                        let mut diagnostic = error.diagnostic;
                        diagnostic.source.get_or_insert(name);
                        diagnostic
                    })
                    .collect();
                Ok(CodeAnalysis {
                    definitions: None,
                    diagnostics,
                    sources,
                })
            }
        }
    }
}
