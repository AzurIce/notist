//! Build-scoped site extensions, called at compilation and template boundaries.
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
/// Keys use `/` separators on every platform, including after filesystem discovery.
/// Native filesystem paths must be converted before entering this map.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileInput {
    pub documents: BTreeMap<PathBuf, String>,
}

/// Final pages are read-only; only template data can be changed.
pub struct TemplateInput<'a> {
    pub pages: &'a [crate::Page],
    pub data: BTreeMap<String, serde_json::Value>,
}

/// Native extensions read explicit resources; command extensions receive the
/// root and config and declare their external file dependencies through watch.
pub struct SiteContext<'a> {
    pub resources: &'a dyn Resources,
    pub config: &'a SiteConfig,
}

/// One mutable instance per build, retained across both events and discarded on
/// success or failure. Builder clones native extension prototypes for each build.
pub trait SiteExtension: Debug + Send + Sync {
    fn name(&self) -> &str;
    fn before_compile(
        &mut self,
        _context: &SiteContext<'_>,
        _input: &mut CompileInput,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Add presentation data after compilation, using final titles and routes.
    fn before_template(
        &mut self,
        _context: &SiteContext<'_>,
        _input: &mut TemplateInput<'_>,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Close build-scoped resources before committing output.
    fn finish(&mut self) -> Result<(), Error> {
        Ok(())
    }
}

pub(crate) trait ExtensionFactory: Debug + Send + Sync {
    fn create(&self) -> Box<dyn SiteExtension>;
}

impl<T: SiteExtension + Clone + 'static> ExtensionFactory for T {
    fn create(&self) -> Box<dyn SiteExtension> {
        Box::new(self.clone())
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
    input: &CompileInput,
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
    Ok(routes)
}

fn validate_data(data: &BTreeMap<String, serde_json::Value>) -> Result<(), Error> {
    for key in data.keys() {
        if !valid_path(Path::new(key)) {
            return Err(Error::Message(format!("invalid site data key `{key}`")));
        }
    }
    Ok(())
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

pub(crate) fn before_compile<R: Resources>(
    vault: &Vault<R>,
    config: &SiteConfig,
    extensions: &mut [Box<dyn SiteExtension>],
    input: &mut CompileInput,
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
    let run = |extension: &mut dyn SiteExtension, input: &mut CompileInput| {
        let progress = notist::progress::Span::stage("site.extensions.before_compile", || {
            format!("Extension {}", extension.name())
        });
        progress.attribute("extension", || extension.name().into());
        let mut candidate = input.clone();
        let routes = extension
            .before_compile(&context, &mut candidate)
            .and_then(|()| validate(vault, config, &candidate))
            .map_err(|error| contextualize(extension, "before_compile", error))?;
        *input = candidate;
        progress.finish();
        Ok::<_, Error>(routes)
    };
    let mut routes = None;
    for extension in extensions {
        routes = Some(run(extension.as_mut(), input)?);
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

pub(crate) fn before_template(
    resources: &dyn Resources,
    config: &SiteConfig,
    extensions: &mut [Box<dyn SiteExtension>],
    input: &mut TemplateInput<'_>,
) -> Result<(), Error> {
    let context = SiteContext { resources, config };
    for extension in extensions {
        let progress = notist::progress::Span::stage("site.extensions.before_template", || {
            format!("Template extension {}", extension.name())
        });
        progress.attribute("extension", || extension.name().into());
        let mut candidate = TemplateInput {
            pages: input.pages,
            data: input.data.clone(),
        };
        extension
            .before_template(&context, &mut candidate)
            .and_then(|()| validate_data(&candidate.data))
            .map_err(|e| contextualize(extension.as_ref(), "before_template", e))?;
        input.data = candidate.data;
        progress.finish();
    }
    Ok(())
}

pub(crate) fn finish(extensions: &mut [Box<dyn SiteExtension>]) -> Result<(), Error> {
    for extension in extensions {
        let progress = notist::progress::Span::stage("site.extensions.finish", || {
            format!("Finish extension {}", extension.name())
        });
        extension
            .finish()
            .map_err(|e| contextualize(extension.as_ref(), "finish", e))?;
        progress.finish();
    }
    Ok(())
}
