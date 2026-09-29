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

    Document,
    Heading,
    Paragraph,
    Raw,
    Inline,
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
        // SAFETY: raw kind 只会来自 kind_to_raw（builder 是唯一的建树入口，
        // 树不落盘、不经外部构造），故取值必为合法判别值。
        unsafe { std::mem::transmute(raw.0) }
    }
}

pub type SyntaxNode = rowan::SyntaxNode<Lang>;
pub type SyntaxToken = rowan::SyntaxToken<Lang>;
