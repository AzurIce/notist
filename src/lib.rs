//! Host runtime foundation for Notist: read-only resource abstraction,
//! bounded execution pool, safe publication planning, diagnostics, and
//! build progress/timing observation.

pub use notist_observe as progress;
pub use notist_observe::timings;

pub mod compiler;
pub mod diag;
pub mod execution;
pub mod inputs;
pub mod publication;
pub mod resources;
pub mod vault;

pub use compiler::{Analyzed, Compiler, CompilerError, Evaluated, SourceSet};
pub use diag::{Diagnostic, DiagnosticCode, Phase, SourceDiagnostic, TextRange, TextSize};
pub use execution::Executor;
pub use inputs::{InputError, InputRead};
pub use resources::{FsResources, MemoryResources, OverlayResources, ResourceError, Resources};
pub use vault::{Vault, VaultError};
