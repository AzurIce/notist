/// What a builtin constructor's children mount accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accepts {
    /// Only inline content (hugging `[..]`).
    Inline,
    /// Any content (an inline mount is promoted to a one-paragraph block).
    Content,
    /// Unconstrained.
    Any,
    /// No children at all (the payload lives in fields).
    Nothing,
}

/// The level of the element a builtin constructor produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// A block-level element; interrupts inline runs.
    Block,
    /// An inline element; flows inside paragraphs.
    Inline,
    /// Transparent: inherits the flavor of its children (`group`).
    Inherit,
}

/// A builtin constructor's signature: mount acceptance and produced level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtorSignature {
    pub accepts: Accepts,
    pub level: Level,
}

/// The builtin constructor table, keyed by source name.
pub const BUILTINS: &[(&str, CtorSignature)] = &[
    ("paragraph", CtorSignature { accepts: Accepts::Inline, level: Level::Block }),
    ("heading", CtorSignature { accepts: Accepts::Inline, level: Level::Block }),
    ("list", CtorSignature { accepts: Accepts::Content, level: Level::Block }),
    ("item", CtorSignature { accepts: Accepts::Content, level: Level::Block }),
    ("strong", CtorSignature { accepts: Accepts::Inline, level: Level::Inline }),
    ("emph", CtorSignature { accepts: Accepts::Inline, level: Level::Inline }),
    ("link", CtorSignature { accepts: Accepts::Inline, level: Level::Inline }),
    ("text", CtorSignature { accepts: Accepts::Nothing, level: Level::Inline }),
    ("raw", CtorSignature { accepts: Accepts::Nothing, level: Level::Inline }),
    ("math", CtorSignature { accepts: Accepts::Nothing, level: Level::Inline }),
    ("group", CtorSignature { accepts: Accepts::Any, level: Level::Inherit }),
];

/// The signature of a builtin constructor by source name (`None` for custom).
pub fn builtin_signature(name: &str) -> Option<CtorSignature> {
    BUILTINS.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// All builtin constructor names.
pub fn builtin_ctors() -> impl Iterator<Item = &'static str> {
    BUILTINS.iter().map(|(n, _)| *n)
}
