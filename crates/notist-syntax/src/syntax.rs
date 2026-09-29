use rowan::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum SyntaxKind {
    Eq = 0,
    Whitespace,
    Text,
    Newline,

    Document,
    Heading,
    Paragraph,
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
        rowan::SyntaxKind(kind as u16)
    }

    fn kind_from_raw(raw: rowan::SyntaxKind) -> Self::Kind {
        match raw.0 {
            0 => SyntaxKind::Eq,
            1 => SyntaxKind::Whitespace,
            2 => SyntaxKind::Text,
            3 => SyntaxKind::Newline,
            4 => SyntaxKind::Document,
            5 => SyntaxKind::Heading,
            6 => SyntaxKind::Paragraph,
            _ => unreachable!(),
        }
    }
}

pub type SyntaxNode = rowan::SyntaxNode<Lang>;
pub type SyntaxToken = rowan::SyntaxToken<Lang>;
