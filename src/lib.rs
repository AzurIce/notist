//! Configurable source frontends producing the shared Notist document IR.
//!
//! ```
//! use notist::{MemoryResources, RenderOptions, Vault};
//!
//! let mut vault = Vault::new(MemoryResources::new("/notes"));
//! let output = vault.render_html("example.md", "# Title\n", RenderOptions::default())?;
//! assert!(output.analysis.diagnostics().is_empty());
//! assert!(!output.rendered.source_map.is_empty());
//! # Ok::<(), notist::VaultError>(())
//! ```
//!
//! [`Pipeline`] accepts explicit source and Context inputs. [`Vault`] combines
//! resource access, declaration environments and rendering; [`PreparedInputs`]
//! transports host inputs to a Worker. Output crates consume core [`Item`].
//! The `notist-cli` crate provides the executable, static-page publication and
//! language-server host. This library has no terminal or LSP dependencies.

pub use rowan::{TextRange, TextSize};

pub use notist_core::annotation::{Annotation, AnnotationKind};
pub use notist_core::definitions::{
    ComptimeParameter, ComptimeValue, DefinitionModule, FunctionDef, FunctionId, FunctionInstance,
    Import, ParameterDef, ParameterMode, Type, ValueConstraint,
};
pub use notist_core::diag::{Diagnostic, DiagnosticCode, Phase, Sources};
pub use notist_core::frontend::{Frontend, FrontendOptions, FrontendOutput, FrontendSyntax};
pub use notist_core::item::{Ctor, Dict, EvaluationState, Item, Origin, SourceLocation, Value};
pub use notist_core::registry::Registry;
pub use notist_core::{annotation, builtins, diag, dump, evaluation, expr, index, item, registry};
pub use notist_pipeline::frontend::{
    PipelineFrontend, markdown::MarkdownFrontend, markup::NotistFrontend,
};
pub use notist_pipeline::{
    Analysis, CodeModule, CodeProgram, ContentAnalysis, ContentLowering, Context, Evaluation,
    Limits, Lowered, Pipeline, UnsupportedFormat,
};
pub use notist_syntax as syntax;

pub mod cst_json;
pub mod json;
pub mod prepared;
pub mod preview;
pub use prepared::PreparedInputs;
pub mod environment;
pub use notist_pipeline::transforms;
pub mod publication;
pub mod query;
pub mod resources;
pub mod vault;
pub mod vault_index;
pub use environment::{Environment, ModuleSource, SourceDiagnostic};
pub use resources::{FsResources, MemoryResources, OverlayResources, ResourceError, Resources};
pub use vault::{CodeAnalysis, HtmlOutput, ItemOutput, RenderOptions, Vault, VaultError};
pub use vault_index::VaultIndex;
#[cfg(target_arch = "wasm32")]
pub mod wasm;
