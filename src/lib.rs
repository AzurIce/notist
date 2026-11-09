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

pub use notist_core::data::{
    Data, DataCache, DataCacheStats, DataError, DataErrorKind, DataFormat, DataLimits,
    ParsedDataCache,
};
pub use notist_core::shared::{Array, Text};
pub use rowan::{TextRange, TextSize};

pub use notist_core::annotation::{Annotation, AnnotationKind, LiteralDict};
pub use notist_core::binding::{BindingId, BindingInfo, Bindings, TypeParamId};
pub use notist_core::definition_identity::{DefinitionError, Definitions};
pub use notist_core::definitions::{
    ComptimeParameter, ComptimeValue, DefId, DefinitionModule, FunctionDef, FunctionId,
    FunctionImpl, FunctionInstance, Import, ImportBinding, ImportKind, ImportMember, ModuleId,
    ModuleRef, NativeFunction, ParameterDef, ParameterMode, ResolvedInstance, Type,
    ValueConstraint,
};
pub use notist_core::diag::{Diagnostic, DiagnosticCode, Phase, Sources};
pub use notist_core::frontend::{
    Frontend, FrontendCacheKey, FrontendOptions, FrontendOutput, FrontendSyntax,
};
pub use notist_core::inputs::{EvaluationInputs, InputError, InputRead};
pub use notist_core::item::{
    ContentItem, Ctor, Dict, Item, Node, NodeContent, Origin, SourceLocation, Value,
};
pub use notist_core::registry::Registry;
pub use notist_core::resource::ResourcePath;
pub use notist_core::source::{
    Located, Source, SourceError, SourceId, SourceNamespace, SourceSpan,
};
pub use notist_core::{
    annotation, builtins, diag, dump, embed, evaluation, expr, index, item, registry,
};
pub use notist_pipeline::frontend::{
    PipelineFrontend, markdown::MarkdownFrontend, markup::NotistFrontend,
};
#[cfg(feature = "profiling")]
pub use notist_pipeline::profiling;
pub use notist_pipeline::progress::{self, timings};
pub use notist_pipeline::{
    Analysis, BindingSpec, CodeModule, CodeProgram, ContentAnalysis, ContentLowering, Context,
    Evaluation, EvaluationOutcome, ExportTable, Limits, LinkedModule, Lowered, ModuleExports,
    ModuleScope, Pipeline, SourceNavigation, UnsupportedFormat, bind_imports, evict_caches,
};
pub use notist_syntax as syntax;

pub mod cst_json;
pub mod json;
pub mod prepared;
pub mod preview;
pub use prepared::{PreparedFailure, PreparedInputs, PreparedResources};
#[cfg(all(not(target_arch = "wasm32"), feature = "command-extensions"))]
pub mod command_extension;
pub mod environment;
#[cfg(all(not(target_arch = "wasm32"), feature = "command-extensions"))]
pub mod stdio;
pub use notist_core::extensions;
pub mod publication;
pub mod query;
pub mod resource_session;
pub mod resources;
pub use resource_session::{InputLimits, ResourceSession};
pub mod vault;
pub mod vault_index;
pub use environment::{Environment, ModuleSource, SourceDiagnostic};
pub use resources::{FsResources, MemoryResources, OverlayResources, ResourceError, Resources};
pub use vault::{
    CodeAnalysis, DocumentHandle, DocumentTooling, HtmlOutput, ItemOutput, RenderOptions, Vault,
    VaultError,
};
pub use vault_index::VaultIndex;
#[cfg(target_arch = "wasm32")]
pub mod wasm;
