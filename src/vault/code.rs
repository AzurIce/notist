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
                    sources: context.program().sources(),
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
                })
            }
        }
    }
}
