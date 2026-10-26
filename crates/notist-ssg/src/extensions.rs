//! Experimental site input extensions, applied once before routing and compilation.
//! The source/data boundary is being validated against concrete generation cases.
use crate::{Error, Route, SiteConfig};
use notist::{Resources, Vault};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt::Debug,
    path::{Path, PathBuf},
};

/// Complete editable site input. Document paths are canonical content-relative
/// paths; values are source text in the format selected by the path's extension.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteInput {
    pub documents: BTreeMap<PathBuf, String>,
    pub data: BTreeMap<String, serde_json::Value>,
}

/// Native extensions read explicit resources; command extensions receive the
/// root and config and declare their external file dependencies through watch.
pub struct SiteContext<'a> {
    pub resources: &'a dyn Resources,
    pub config: &'a SiteConfig,
}

/// Experimental site extension. Runs once per build after discovery and data
/// loading, before documents enter the shared Notist pipeline. It can add,
/// replace or remove sources and data. A failed extension aborts the build.
pub trait SiteExtension: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn prepare(&self, context: &SiteContext<'_>, input: &mut SiteInput) -> Result<(), Error>;
}

/// Configuration of a site-level extension, distinct from document extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum ExtensionConfig {
    Command {
        command: Vec<String>,
        /// Paths relative to the site root, observed even if the command fails.
        #[serde(default)]
        watch: Vec<PathBuf>,
    },
}

pub(crate) fn valid_path(path: &Path) -> bool {
    path.to_str().is_some_and(|s| {
        !s.is_empty()
            && !s.contains(['\\', '\0'])
            && s.split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
    }) && !path.is_absolute()
}

fn validate<R: Resources>(
    vault: &Vault<R>,
    content: &Path,
    input: &SiteInput,
) -> Result<(), Error> {
    let mut outputs = BTreeMap::new();
    for path in input.documents.keys() {
        if !valid_path(path)
            || path
                .components()
                .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
            || !vault.supports(path)
        {
            return Err(Error::Message(format!(
                "invalid site document path `{}`",
                path.display()
            )));
        }
        let route = Route::new(content.join(path), content)?;
        if outputs.insert(route.output.clone(), path).is_some() {
            return Err(Error::Message(format!(
                "route collision at `{}`",
                route.output.display()
            )));
        }
    }
    for key in input.data.keys() {
        if !valid_path(Path::new(key)) {
            return Err(Error::Message(format!("invalid site data key `{key}`")));
        }
    }
    Ok(())
}

fn contextualize(extension: &dyn SiteExtension, error: Error) -> Error {
    let prefix = format!("site extension `{}` (prepare)", extension.name());
    match error {
        Error::Diagnostics(mut errors) => {
            for error in &mut errors {
                error.diagnostic.message = format!("{prefix}: {}", error.diagnostic.message);
            }
            Error::Diagnostics(errors)
        }
        error => Error::Message(format!("{prefix}: {error}")),
    }
}

pub(crate) fn prepare<R: Resources>(
    vault: &Vault<R>,
    config: &SiteConfig,
    native: &[Box<dyn SiteExtension>],
    input: &mut SiteInput,
) -> Result<(), Error> {
    let content = vault.resources().resolve(&config.content);
    let context = SiteContext {
        resources: vault.resources(),
        config,
    };
    // Register external inputs before running hooks so failed builds still watch
    // files that can fix the command. Resources records observations for hosts.
    for extension in &config.extensions {
        let ExtensionConfig::Command { watch, .. } = extension;
        for path in watch {
            vault.resources().kind(path)?;
        }
    }
    let run = |extension: &dyn SiteExtension, input: &mut SiteInput| {
        let mut candidate = input.clone();
        extension
            .prepare(&context, &mut candidate)
            .and_then(|()| validate(vault, &content, &candidate))
            .map_err(|error| contextualize(extension, error))?;
        *input = candidate;
        Ok::<_, Error>(())
    };
    for extension in native {
        run(extension.as_ref(), input)?;
    }
    for extension in &config.extensions {
        let ExtensionConfig::Command { command, .. } = extension;
        #[cfg(not(target_arch = "wasm32"))]
        run(
            &super::command::CommandExtension::new(command.clone()),
            input,
        )?;
        #[cfg(target_arch = "wasm32")]
        {
            let _ = command;
            return Err(Error::Message(
                "site command extensions are unavailable in this host".into(),
            ));
        }
    }
    validate(vault, &content, input)
}
