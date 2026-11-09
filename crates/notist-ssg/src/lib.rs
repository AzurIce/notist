//! Directory-preserving document publication with replaceable site templates.
mod assets;
mod build;
pub mod builtins;
#[cfg(not(target_arch = "wasm32"))]
pub mod command;
mod config;
mod document;
pub mod extensions;
mod mounts;
mod routes;
mod templates;
mod themes;

pub use build::BuildOutput;
pub use config::{Mount, SiteConfig};
pub use document::{Heading, Page};
pub use notist::Executor;
pub use routes::Route;
use std::sync::Arc;

/// Site build driver, with native extensions followed by configured commands.
#[derive(Debug, Default)]
pub struct Builder {
    extensions: Vec<Box<dyn extensions::ExtensionFactory>>,
    executor: Option<Arc<Executor>>,
}
impl Builder {
    /// Share one concurrency budget across stages, package builds and rebuilds.
    /// Without an explicit executor, each build uses the available CPU count.
    ///
    /// ```
    /// use notist_ssg::{Builder, Executor};
    /// use std::{num::NonZeroUsize, sync::Arc};
    /// let executor = Arc::new(Executor::new(NonZeroUsize::new(4).unwrap())?);
    /// let builder = Builder::default().with_executor(executor);
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn with_executor(mut self, executor: Arc<Executor>) -> Self {
        self.executor = Some(executor);
        self
    }

    fn executor(&self) -> Result<Arc<Executor>, Error> {
        match &self.executor {
            Some(executor) => Ok(executor.clone()),
            None => Executor::auto()
                .map(Arc::new)
                .map_err(|error| Error::Message(format!("cannot create build executor: {error}"))),
        }
    }

    /// Register a prototype; each build receives a fresh clone, not the previous
    /// build's mutated instance. Clone should isolate build-local mutable state.
    pub fn with_extension(
        mut self,
        extension: impl extensions::SiteExtension + Clone + 'static,
    ) -> Self {
        self.extensions.push(Box::new(extension));
        self
    }
    pub fn build<R: notist::Resources + Send>(
        &self,
        vault: &mut notist::Vault<R>,
        config: &SiteConfig,
    ) -> Result<BuildOutput, Error> {
        let executor = self.executor()?;
        build::build(vault, config, &self.extensions, false, &executor)
    }

    /// Preview retains recoverable content errors, but never publishes fatal failures.
    pub fn serve<R: notist::Resources + Send>(
        &self,
        vault: &mut notist::Vault<R>,
        config: &SiteConfig,
    ) -> Result<BuildOutput, Error> {
        let executor = self.executor()?;
        build::build(vault, config, &self.extensions, true, &executor)
    }
}

#[derive(Debug)]
pub enum Error {
    Diagnostics(Vec<notist::SourceDiagnostic>),
    Message(String),
    Resource(notist::resources::ResourceError),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Diagnostics(errors) => {
                for error in errors {
                    writeln!(f, "{}: {}", error.path.display(), error.diagnostic.message)?;
                }
                Ok(())
            }
            Self::Message(message) => f.write_str(message),
            Self::Resource(error) => error.fmt(f),
        }
    }
}
impl std::error::Error for Error {}
impl From<notist::resources::ResourceError> for Error {
    fn from(error: notist::resources::ResourceError) -> Self {
        Self::Resource(error)
    }
}
impl From<notist::VaultError> for Error {
    fn from(error: notist::VaultError) -> Self {
        match error {
            notist::VaultError::Environment(errors) => Self::Diagnostics(errors),
            notist::VaultError::Resource(error) => Self::Resource(error),
            error => Self::Message(error.to_string()),
        }
    }
}
impl From<notist::publication::Error> for Error {
    fn from(error: notist::publication::Error) -> Self {
        match error {
            notist::publication::Error::Resource(error) => Self::Resource(error),
            error => Self::Message(error.to_string()),
        }
    }
}
