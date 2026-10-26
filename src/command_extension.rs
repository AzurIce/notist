//! Experimental native stdio adapter for document extension stages.
//!
//! Protocol version 1 identifies this draft format; it is not a stability guarantee.
use crate::environment::DocumentStage;
use crate::extensions::{DocumentContext, DocumentExtension, ExtensionResult, LoweredDocument};
use crate::{Diagnostic, FunctionDef, Item, Phase};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// One JSON request and response per process invocation. Stdout contains only
/// the response; stderr is forwarded to the calling host for command logs.
pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct Request<T> {
    pub version: u32,
    pub stage: DocumentStage,
    pub path: String,
    pub sources: BTreeMap<String, String>,
    pub functions: Vec<FunctionDef>,
    pub document: T,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    /// Omit to keep the current value. `after_check` cannot return a document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<serde_json::Value>,
    /// Every diagnostic fails the hook and rolls back its candidate edits.
    #[serde(default)]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone)]
pub struct CommandExtension {
    command: Vec<String>,
    stages: Vec<DocumentStage>,
    directory: PathBuf,
}
impl CommandExtension {
    /// Arguments are passed directly; paths and file reads are relative to directory.
    pub fn new(command: Vec<String>, stages: Vec<DocumentStage>, directory: PathBuf) -> Self {
        Self {
            command,
            stages,
            directory,
        }
    }
    fn run<T: Serialize>(
        &self,
        stage: DocumentStage,
        context: &DocumentContext<'_>,
        document: &T,
    ) -> Result<Option<serde_json::Value>, Vec<Diagnostic>> {
        if !self.stages.contains(&stage) {
            return Ok(None);
        }
        let request = Request {
            version: PROTOCOL_VERSION,
            stage,
            path: context.path.into(),
            sources: context
                .sources
                .iter()
                .map(|(path, source)| (path.clone(), source.to_string()))
                .collect(),
            functions: context.registry.functions().cloned().collect(),
            document,
        };
        let response: Response = crate::stdio::exchange(&self.command, &self.directory, &request)
            .map_err(|e| failure(e.to_string()))?;
        if response.version != PROTOCOL_VERSION {
            return Err(failure(format!(
                "unsupported response version {}",
                response.version
            )));
        }
        if !response.diagnostics.is_empty() {
            return Err(response.diagnostics);
        }
        Ok(response.document)
    }
}
impl DocumentExtension for CommandExtension {
    fn name(&self) -> &str {
        self.command
            .first()
            .map(String::as_str)
            .unwrap_or("command")
    }
    fn validate(&self, _: &crate::Registry) -> ExtensionResult {
        if self
            .command
            .first()
            .is_none_or(|program| program.is_empty())
        {
            return Err(failure("command requires a nonempty program".into()));
        }
        if self.stages.is_empty()
            || self
                .stages
                .iter()
                .enumerate()
                .any(|(i, stage)| self.stages[..i].contains(stage))
        {
            return Err(failure("stages must be nonempty and unique".into()));
        }
        Ok(())
    }
    fn before_check(
        &self,
        context: &DocumentContext<'_>,
        document: &mut LoweredDocument,
    ) -> ExtensionResult {
        if let Some(value) = self.run(DocumentStage::BeforeCheck, context, document)? {
            *document = serde_json::from_value(value)
                .map_err(|e| failure(format!("invalid lowered document: {e}")))?;
        }
        Ok(())
    }
    fn after_check(&self, context: &DocumentContext<'_>, document: &Item) -> ExtensionResult {
        if self
            .run(DocumentStage::AfterCheck, context, document)?
            .is_some()
        {
            return Err(failure(
                "after_check is read-only; omit document in the response".into(),
            ));
        }
        Ok(())
    }
    fn after_evaluate(
        &self,
        context: &DocumentContext<'_>,
        document: &mut Item,
    ) -> ExtensionResult {
        if let Some(value) = self.run(DocumentStage::AfterEvaluate, context, document)? {
            *document = serde_json::from_value(value)
                .map_err(|e| failure(format!("invalid output document: {e}")))?;
        }
        Ok(())
    }
}
fn failure(message: String) -> Vec<Diagnostic> {
    vec![Diagnostic::new(
        Phase::Semantic,
        Default::default(),
        message,
    )]
}
