//! Resource-backed document vault for site builds and host tooling.
use crate::resources::{ResourceError, Resources};
use std::{fmt, path::Path};

#[derive(Debug)]
pub enum VaultError {
    Resource(ResourceError),
    Message(String),
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Resource(err) => err.fmt(f),
            Self::Message(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for VaultError {}

impl From<ResourceError> for VaultError {
    fn from(err: ResourceError) -> Self {
        Self::Resource(err)
    }
}

/// Host resource handle carrying supported document format checks.
#[derive(Debug)]
pub struct Vault<R> {
    resources: R,
}

impl<R: Resources> Vault<R> {
    pub fn new(resources: R) -> Self {
        Self { resources }
    }

    pub fn resources(&self) -> &R {
        &self.resources
    }

    /// Whether `path` has a supported document extension (`.md` or `.markdown`).
    pub fn supports(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|ext| {
                ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown")
            })
    }

    /// Read a UTF-8 source document through the underlying resource backend.
    pub fn source(&self, path: &Path) -> Result<String, VaultError> {
        let bytes = self.resources.read(path)?;
        String::from_utf8(bytes)
            .map_err(|_| VaultError::Resource(ResourceError::InvalidUtf8(path.to_path_buf())))
    }
}
