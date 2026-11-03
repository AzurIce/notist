//! A narrow synchronous view of host-owned, immutable compilation inputs.
//! Implementations own authorization, bounded loading and per-batch caching.
use crate::resource::ResourcePath;
use std::{fmt, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum InputError {
    NotFound,
    AccessDenied,
    NotFile,
    TooLarge,
    CacheLimit,
    RequestLimit,
    Unavailable,
    Io(String),
}
impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "resource not found",
            Self::AccessDenied => "resource access denied",
            Self::NotFile => "resource is not a regular file",
            Self::TooLarge => "resource size limit exceeded",
            Self::CacheLimit => "resource session cache limit exceeded",
            Self::RequestLimit => "resource session request limit exceeded",
            Self::Unavailable => "resource reading capability is unavailable",
            Self::Io(message) => message,
        })
    }
}
impl std::error::Error for InputError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputRead {
    Ready(Arc<[u8]>),
    Failed(InputError),
    /// The host must prepare this input and retry with a new immutable view.
    Pending,
}

pub trait EvaluationInputs {
    fn file(&self, path: &ResourcePath) -> InputRead;
    fn data_cache(&self) -> Option<&dyn crate::data::ParsedDataCache> {
        None
    }
}
