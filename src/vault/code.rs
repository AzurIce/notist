//! Code tooling uses the host's project context and explicit source overlays.
use super::*;
use crate::{Context, DefinitionModule, Diagnostic, OverlayResources};

#[derive(Debug)]
pub struct CodeAnalysis {
    pub definitions: Option<DefinitionModule>,
    pub diagnostics: Vec<Diagnostic>,
    pub sources: BTreeMap<String, String>,
}

impl<R: Resources> Vault<R> {
    /// Check a native-Markup Code source in its actual project. The supplied
    /// source overlays resources, including when editing the package entry.
    /// Global load failures retain their own source identities and ranges.
    pub fn analyze_code(
        &self,
        path: impl AsRef<Path>,
        source: &str,
    ) -> Result<CodeAnalysis, VaultError> {
        let path = self.resources.resolve(path.as_ref());
        let name = path.to_string_lossy().into_owned();
        let overlay = OverlayResources::new(
            &self.resources,
            &BTreeMap::from([(path.clone(), source.into())]),
        );
        let config = match &self.config {
            Some(path) => Some(path.clone()),
            None => discover_config_in(&overlay, &path)?,
        };
        let context = match config {
            Some(config) => match Environment::load_from(&overlay, config) {
                Ok(environment) => environment.context().clone(),
                Err(errors) => {
                    let mut sources = BTreeMap::from([(name, source.into())]);
                    let diagnostics = errors
                        .into_iter()
                        .map(|error| {
                            let path = error.path.to_string_lossy().into_owned();
                            sources.insert(path.clone(), error.source);
                            let mut diagnostic = error.diagnostic;
                            diagnostic.source.get_or_insert(path);
                            diagnostic
                        })
                        .collect();
                    return Ok(CodeAnalysis {
                        definitions: None,
                        diagnostics,
                        sources,
                    });
                }
            },
            None => self
                .environments
                .get(&None)
                .map(|e| e.context().clone())
                .unwrap_or_else(Context::default),
        };
        let package = context
            .program()
            .package_for_source(&name)
            .unwrap_or("package")
            .to_owned();
        let mut sources = context.program().sources().clone();
        sources.insert(name.clone(), source.into());
        let module = match self
            .pipeline
            .compile_module("content.not", &package, &name, source)
        {
            Ok(module) => module,
            Err(mut diagnostics) => {
                for d in &mut diagnostics {
                    d.source.get_or_insert(name.clone());
                }
                return Ok(CodeAnalysis {
                    definitions: None,
                    diagnostics,
                    sources,
                });
            }
        };
        let definitions = Some(module.definitions.clone());
        let installed = context.program().source_path(&package).is_some();
        let context = if installed {
            context.replace_module(module)
        } else {
            context.with_module(module)
        };
        let diagnostics = match context {
            Ok(context) => self.pipeline.check_module(&package, &context),
            Err(diagnostics) => diagnostics,
        };
        Ok(CodeAnalysis {
            definitions,
            diagnostics,
            sources,
        })
    }
}
