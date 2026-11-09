//! Experimental site input extensions, applied once before routing and compilation.
//! The source/data boundary is being validated against concrete generation cases.
use crate::{Error, Route, SiteConfig};
use notist::{Resources, Vault};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
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

    /// Add presentation data after compilation, using final titles and routes.
    fn enrich(
        &self,
        _context: &SiteContext<'_>,
        _pages: &[crate::Page],
        _data: &mut BTreeMap<String, serde_json::Value>,
    ) -> Result<(), Error> {
        Ok(())
    }
}

/// Named extensions. `builtins` is a reserved namespace, not an instance name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtensionsConfig {
    #[serde(default)]
    pub builtins: BuiltinsConfig,
    #[serde(flatten)]
    pub commands: BTreeMap<String, CommandConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltinsConfig {
    pub book: Option<crate::builtins::book::BookConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandConfig {
    pub command: Vec<String>,
    #[serde(default)]
    pub watch: Vec<PathBuf>,
}

impl ExtensionsConfig {
    pub fn is_empty(&self) -> bool {
        self.builtins.book.is_none() && self.commands.is_empty()
    }

    pub(crate) fn names(&self) -> BTreeSet<String> {
        self.commands
            .keys()
            .cloned()
            .chain(
                self.builtins
                    .book
                    .as_ref()
                    .map(|_| "builtins.book".to_owned()),
            )
            .collect()
    }
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
    config: &SiteConfig,
    input: &SiteInput,
) -> Result<Vec<Route>, Error> {
    let mut outputs = BTreeSet::new();
    let content = vault.resources().resolve(&config.content);
    let mounts = crate::mounts::Mounts::new(vault.resources(), config)?;
    let mut routes = Vec::with_capacity(input.documents.len());
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
        let mut route = Route::plan(content.join(path), &content, config.directory_urls)?;
        route.source = mounts.resolve(&route.source);
        if !outputs.insert(route.output.clone()) {
            return Err(Error::Message(format!(
                "route collision at `{}`",
                route.output.display()
            )));
        }
        routes.push(route);
    }
    for key in input.data.keys() {
        if !valid_path(Path::new(key)) {
            return Err(Error::Message(format!("invalid site data key `{key}`")));
        }
    }
    Ok(routes)
}

fn contextualize(extension: &dyn SiteExtension, stage: &str, error: Error) -> Error {
    let prefix = format!("site extension `{}` ({stage})", extension.name());
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
    extensions: &[&dyn SiteExtension],
    input: &mut SiteInput,
) -> Result<Vec<Route>, Error> {
    let context = SiteContext {
        resources: vault.resources(),
        config,
    };
    // Register external inputs before running hooks so failed builds still watch
    // files that can fix the command. Resources records observations for hosts.
    for extension in config.extensions.commands.values() {
        for path in &extension.watch {
            vault.resources().kind(path)?;
        }
    }
    let run = |extension: &dyn SiteExtension, input: &mut SiteInput| {
        let progress = notist::progress::Span::enter(|| format!("Extension {}", extension.name()));
        let mut candidate = input.clone();
        let routes = extension
            .prepare(&context, &mut candidate)
            .and_then(|()| validate(vault, config, &candidate))
            .map_err(|error| contextualize(extension, "prepare", error))?;
        *input = candidate;
        progress.finish();
        Ok::<_, Error>(routes)
    };
    let mut routes = None;
    for extension in extensions {
        routes = Some(run(*extension, input)?);
    }
    match routes {
        Some(routes) => Ok(routes),
        None => validate(vault, config, input),
    }
}

pub(crate) fn configured(config: &SiteConfig) -> Result<Vec<Box<dyn SiteExtension>>, Error> {
    let names = if config.extension_order.is_empty() {
        config.extensions.names().into_iter().collect()
    } else {
        config.extension_order.clone()
    };
    names
        .into_iter()
        .map(|name| {
            if name == "builtins.book" {
                return Ok(Box::new(crate::builtins::book::Book::new(
                    config.extensions.builtins.book.clone().unwrap(),
                )) as Box<dyn SiteExtension>);
            }
            let command = &config.extensions.commands[&name];
            #[cfg(not(target_arch = "wasm32"))]
            {
                Ok(Box::new(super::command::CommandExtension::named(
                    name,
                    command.command.clone(),
                )) as Box<dyn SiteExtension>)
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = command;
                Err(Error::Message(
                    "site command extensions are unavailable in this host".into(),
                ))
            }
        })
        .collect()
}

pub(crate) fn enrich(
    resources: &dyn Resources,
    config: &SiteConfig,
    extensions: &[&dyn SiteExtension],
    pages: &[crate::Page],
    data: &mut BTreeMap<String, serde_json::Value>,
) -> Result<(), Error> {
    let context = SiteContext { resources, config };
    for extension in extensions {
        let mut candidate = data.clone();
        extension
            .enrich(&context, pages, &mut candidate)
            .map_err(|e| contextualize(*extension, "enrich", e))?;
        *data = candidate;
    }
    Ok(())
}
