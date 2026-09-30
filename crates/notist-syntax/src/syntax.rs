use rowan::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    Eq = 0,
    Whitespace,
    Text,
    Newline,
    Backtick,
    LineComment,
    BlockComment,
    Star,
    Underscore,
    Backslash,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Pipe,
    Hash,
    Dollar,
    Minus,
    Plus,
    At,
    Bang,
    Str,
    Number,
    Ident,
    Colon,
    Comma,

    Document,
    Heading,
    Paragraph,
    Raw,
    RawInline,
    Math,
    Inline,
    Link,
    WikiLink,
    Strong,
    Emph,
    List,
    ListItem,
    Annotation,
    CodeCall,
    Entry,
    Dict,
    Array,
    Unit,
    Neg,
    Escape,
    ParBreak,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Lang;

impl From<SyntaxKind> for rowan::SyntaxKind {
    fn from(kind: SyntaxKind) -> Self {
        rowan::SyntaxKind(kind as u16)
    }
}

impl Language for Lang {
    type Kind = SyntaxKind;

    fn kind_to_raw(kind: Self::Kind) -> rowan::SyntaxKind {
        kind.into()
    }

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        // SAFETY: raw kinds only ever come from kind_to_raw (the builder is
        // the sole tree constructor; trees are never persisted or produced
        // elsewhere), so the value is always a valid discriminant.
        unsafe { std::mem::transmute(raw.0) }
    }
}

pub type SyntaxNode = rowan::SyntaxNode<Lang>;
pub type SyntaxToken = rowan::SyntaxToken<Lang>;
