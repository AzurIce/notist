use rowan::TextRange;

/// Which pipeline phase produced a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Parser: malformed structure.
    Syntax,
    /// Desugar: value-domain errors (bare names, number ranges, entries).
    Semantic,
    /// Definition validation and Resolve: signature/flavor mismatches.
    Type,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Phase::Syntax => "syntax",
            Phase::Semantic => "semantic",
            Phase::Type => "type",
        })
    }
}

/// The shared diagnostic type of the whole pipeline: `notist-syntax`'s own
/// `parser::Diagnostic` is converted in at the boundary with `phase: Syntax`.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub span: TextRange,
    pub message: String,
    pub phase: Phase,
    /// Source identity when expansion/checking crosses the document boundary.
    pub source: Option<String>,
    /// A content boundary contains this failure and preserves independent content.
    /// Hosts may present the recovered tree regardless of the diagnostic phase.
    pub recovered: bool,
}

impl Diagnostic {
    pub fn new(phase: Phase, span: TextRange, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
            phase,
            source: None,
            recovered: false,
        }
    }
}
