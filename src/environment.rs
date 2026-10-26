//! Host assembly of local Code modules and their relative component resources.
use crate::resources::{ResourceKind, Resources};
use crate::{Context, Diagnostic, TextRange};
use std::collections::BTreeMap;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
mod config;
mod loader;
pub use config::{Configuration, DocumentStage, ExtensionBinding, parse_config};

#[derive(Debug, Clone)]
pub struct SourceDiagnostic {
    pub path: PathBuf,
    pub source: String,
    pub diagnostic: Diagnostic,
}
#[derive(Debug, Clone)]
pub struct ModuleSource {
    pub path: PathBuf,
    pub source: Arc<str>,
}
#[derive(Debug)]
pub struct Environment {
    config_path: Option<PathBuf>,
    modules: BTreeMap<String, ModuleSource>,
    context: Context,
}
impl Environment {
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }
    pub fn modules(&self) -> &BTreeMap<String, ModuleSource> {
        &self.modules
    }
    pub fn context(&self) -> &Context {
        &self.context
    }
    /// Assemble the document's own import scope and explicit output mappings.
    pub(crate) fn for_document(
        resources: &(impl Resources + ?Sized),
        pipeline: &crate::Pipeline,
        path: &Path,
        source: &str,
        imports: &[crate::Import],
        config: Option<&Path>,
        extensions: &[Arc<dyn crate::extensions::DocumentExtension>],
    ) -> Result<Self, Vec<SourceDiagnostic>> {
        loader::document(
            resources, pipeline, path, source, imports, config, extensions,
        )
    }
    /// Load a Code source and its explicit import graph.
    pub fn load_from(
        resources: &(impl Resources + ?Sized),
        entry: impl AsRef<Path>,
    ) -> Result<Self, Vec<SourceDiagnostic>> {
        loader::load(resources, entry.as_ref())
    }
}

/// Discover the nearest output configuration within the Vault root.
pub fn discover_config_in(
    resources: &(impl Resources + ?Sized),
    document: &Path,
) -> Result<Option<PathBuf>, crate::resources::ResourceError> {
    let document = resources.resolve(document);
    let directory = if resources.kind(&document)? == Some(ResourceKind::Directory) {
        document.as_path()
    } else {
        document.parent().unwrap_or(resources.root())
    };
    for directory in directory.ancestors() {
        let path = directory.join("Notist.toml");
        if resources.kind(&path)? == Some(ResourceKind::File) {
            return Ok(Some(path));
        }
        if directory == resources.root() {
            break;
        }
    }
    Ok(None)
}
fn range(span: std::ops::Range<usize>) -> TextRange {
    TextRange::new((span.start as u32).into(), (span.end as u32).into())
}
pub(crate) fn issue(
    path: &Path,
    source: &str,
    span: TextRange,
    message: String,
) -> SourceDiagnostic {
    SourceDiagnostic {
        path: path.into(),
        source: source.into(),
        diagnostic: Diagnostic::new(crate::Phase::Semantic, span, message),
    }
}
