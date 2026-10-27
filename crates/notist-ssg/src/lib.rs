//! Directory-preserving document publication with replaceable site templates.
mod assets;
mod build;
#[cfg(not(target_arch = "wasm32"))]
pub mod command;
mod config;
mod document;
pub mod extensions;
mod routes;
mod templates;

pub use build::BuildOutput;
pub use config::SiteConfig;
pub use document::{Heading, Page};
pub use routes::Route;

/// Site build driver, with native extensions followed by configured commands.
#[derive(Debug, Default)]
pub struct Builder {
    extensions: Vec<Box<dyn extensions::SiteExtension>>,
}
impl Builder {
    pub fn with_extension(mut self, extension: impl extensions::SiteExtension + 'static) -> Self {
        self.extensions.push(Box::new(extension));
        self
    }
    pub fn build<R: notist::Resources>(
        &self,
        vault: &mut notist::Vault<R>,
        config: &SiteConfig,
    ) -> Result<BuildOutput, Error> {
        build::build(vault, config, &self.extensions)
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
