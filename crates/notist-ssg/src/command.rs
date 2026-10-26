//! Experimental site stdio protocol. This exchanges source text and JSON data,
//! independently of the experimental document IR protocol.
use crate::{
    Error, SiteConfig,
    extensions::{SiteContext, SiteExtension, SiteInput},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    pub stage: String,
    pub root: PathBuf,
    pub config: SiteConfig,
    pub input: SiteInput,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<SiteInput>,
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CommandExtension {
    command: Vec<String>,
}
impl CommandExtension {
    pub fn new(command: Vec<String>) -> Self {
        Self { command }
    }
}
impl SiteExtension for CommandExtension {
    fn name(&self) -> &str {
        self.command
            .first()
            .map(String::as_str)
            .unwrap_or("command")
    }
    fn prepare(&self, context: &SiteContext<'_>, input: &mut SiteInput) -> Result<(), Error> {
        let request = Request {
            version: PROTOCOL_VERSION,
            stage: "prepare".into(),
            root: context.resources.root().into(),
            config: context.config.clone(),
            input: input.clone(),
        };
        let response: Response =
            notist::stdio::exchange(&self.command, context.resources.root(), &request)
                .map_err(|e| Error::Message(e.to_string()))?;
        if response.version != PROTOCOL_VERSION {
            return Err(Error::Message(format!(
                "unsupported site extension response version {}",
                response.version
            )));
        }
        if !response.errors.is_empty() {
            return Err(Error::Message(response.errors.join("\n")));
        }
        if let Some(value) = response.input {
            *input = value;
        }
        Ok(())
    }
}
