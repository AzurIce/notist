//! Common Rowan representation for the Markup and Code syntax domains.
//! Each domain defines its own typed AST and roots over this vocabulary.
use rowan::Language;

// Keep the raw vocabulary and its checked conversion in one declaration.
macro_rules! syntax_kinds {
    ($($(#[$meta:meta])* $kind:ident),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        pub enum SyntaxKind {
            $($(#[$meta])* $kind),*
        }

        impl SyntaxKind {
            const ALL: &[Self] = &[$(Self::$kind),*];
        }
    };
}

syntax_kinds! {
    Eq,
    Whitespace,
    Text,
    Newline,
    Backtick,
    LineComment,
    BlockComment,
    Star,
    Underscore,
    Tilde,
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
    Escape,

    Document,
    Heading,
    Raw,
    RawInline,
    Math,
    Link,
    Embed,
    WikiLink,
    Strong,
    Emph,
    Strike,
    Divider,
    Table,
    TableRow,
    TableCell,
    TableDelimiter,
    List,
    ListItem,
    Annotation,
    /// An annotation parsed at a block boundary (a standalone line).
    BlockAnnotation,
    CodeCall,
    Entry,
    Dict,
    Array,
    Unit,
    Neg,
    Group,
    ParBreak,
    Error,

    // Code declarations, paths and content-literal frames.
    Dot,
    FnKeyword,
    ImportKeyword,
    AsKeyword,
    ImportDecl,
    ImportMember,
    MemberExpr,
    Question,
    Arrow,
    Semicolon,
    Less,
    Greater,
    Module,
    FunctionDecl,
    ParameterList,
    Parameter,
    ChildrenDecl,
    TypeRef,
    DefaultValue,
    Path,
    ContentLiteral,
    ContentText,
    LBrace,
    RBrace,
    EqEq,
    NotEq,
    LessEq,
    GreaterEq,
    AndAnd,
    OrOr,
    Slash,
    Percent,
    FatArrow,
    LetKeyword,
    IfKeyword,
    ElseKeyword,
    MatchKeyword,
    ComptimeKeyword,
    ComptimeParameters,
    ComptimeArguments,
    ArgumentList,
    BlockExpr,
    LetBinding,
    IfExpr,
    MatchExpr,
    MatchArm,
    BinaryExpr,
    UnaryExpr,
    InvokeExpr,
    ComptimeExpr,
    /// A resource path literal, distinct from a dotted name `Path`.
    PathLiteral,
    ForKeyword,
    InKeyword,
    ForExpr,
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
        SyntaxKind::ALL
            .get(usize::from(raw.0))
            .copied()
            .unwrap_or_else(|| panic!("invalid syntax kind {}", raw.0))
    }
}

pub type SyntaxNode = rowan::SyntaxNode<Lang>;
pub type SyntaxToken = rowan::SyntaxToken<Lang>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_round_trips_without_changing_raw_numbers() {
        for (raw, &kind) in SyntaxKind::ALL.iter().enumerate() {
            assert_eq!(kind as usize, raw);
            assert_eq!(Lang::kind_from_raw(Lang::kind_to_raw(kind)), kind);
        }
    }

    #[test]
    fn invalid_raw_kinds_fail_safely() {
        for raw in [SyntaxKind::ALL.len() as u16, u16::MAX] {
            assert!(
                std::panic::catch_unwind(|| Lang::kind_from_raw(rowan::SyntaxKind(raw))).is_err()
            );
        }
    }
}
