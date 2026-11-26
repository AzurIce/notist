//! Source-level declarations and expressions, before name or type resolution.

/// Synthetic entry function name generated for `.not` document files.
pub const DOC_ENTRY: &str = "__doc__";

/// Byte range `[start, end)` within a single source string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub const DUMMY: Self = Self { start: 0, end: 0 };

    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    pub const fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }

    pub const fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }

    pub const fn contains_inclusive(self, offset: u32) -> bool {
        self.start <= offset && offset <= self.end
    }

    pub fn merge(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Debug)]
pub struct Program {
    /// The only module when compiling a standalone single-file program.
    pub root: Module,
}

#[derive(Debug)]
pub struct Module {
    pub declarations: Vec<Decl>,
}

#[derive(Debug)]
pub enum Decl {
    Import(Import),
    Function(Function),
    Method(Method),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    pub path: String,
    pub path_span: Span,
    pub kind: ImportKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportKind {
    /// `import "./math.not";` — alias derived from file stem.
    Default,
    /// `import "./math.not" as m;`
    Alias { alias: String, span: Span },
    /// `import "./math.not": inc, add as sum;`
    Members(Vec<ImportMember>),
    /// `import "./math.not": *;`
    Wildcard { span: Span },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportMember {
    pub name: String,
    pub name_span: Span,
    pub alias: String,
    pub alias_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    Unit,
    Bool,
    Int,
    Float,
    String,
    Inline,
    Block,
    Json,
    List(Box<Type>),
    Never,
    Param(String),
    Function(Vec<Type>, Box<Type>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeParam {
    pub name: String,
    pub default: Option<Type>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct WhereExpr {
    pub kind: WhereExprKind,
    pub span: Span,
}

impl WhereExpr {
    pub const fn new(kind: WhereExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone)]
pub enum WhereExprKind {
    Bool(bool),
    TypeEq(Type, Type),
    TypeNe(Type, Type),
    Not(Box<WhereExpr>),
    And(Box<WhereExpr>, Box<WhereExpr>),
    Or(Box<WhereExpr>, Box<WhereExpr>),
}

#[derive(Debug)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug)]
pub struct Function {
    pub name: String,
    pub name_span: Span,
    pub type_params: Vec<TypeParam>,
    pub parameters: Vec<Param>,
    pub returns: Type,
    pub where_clause: Option<WhereExpr>,
    /// `None` for bodyless external content declarations (`fn badge(s: String) -> Inline;`).
    pub body: Option<Expr>,
    pub span: Span,
}

#[derive(Debug)]
pub struct Method {
    pub receiver_type: Type,
    pub name: String,
    pub name_span: Span,
    pub receiver_name: String,
    pub receiver_span: Span,
    pub parameters: Vec<Param>,
    pub returns: Type,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Neg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Mul,
    Div,
    Rem,
}

#[derive(Debug)]
pub struct TypeMatchArm {
    pub pattern: TypePattern,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypePattern {
    Type(Type),
    Wildcard,
}

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub const fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttrLiteral {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<AttrLiteral>),
}

#[derive(Debug)]
pub struct AttrEntry {
    pub key: String,
    pub value: AttrLiteral,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableAlign {
    #[default]
    None,
    Left,
    Center,
    Right,
}

#[derive(Debug)]
pub enum InlinePiece {
    /// Unattached inline attribute annotation `@(...)` waiting for the unified
    /// AST-level attribute attachment pass.
    Attr {
        attrs: Vec<AttrEntry>,
        span: Span,
    },
    Text(String, Span),
    Strong(Vec<InlinePiece>, Span),
    Emph(Vec<InlinePiece>, Span),
    Strike(Vec<InlinePiece>, Span),
    Raw(String, Span),
    Link {
        target: String,
        label: Vec<InlinePiece>,
        span: Span,
    },
    Image {
        target: String,
        alt: String,
        span: Span,
    },
    Math(String, Span),
    Expr(Box<Expr>, Span),
    Attributed {
        attrs: Vec<AttrEntry>,
        target: Box<InlinePiece>,
        span: Span,
    },
}

#[derive(Debug)]
pub enum BlockPiece {
    /// Unattached block-level attribute annotation `@(...)` waiting for the
    /// unified AST-level attribute attachment pass.
    Attr {
        attrs: Vec<AttrEntry>,
        span: Span,
    },
    Paragraph {
        inlines: Vec<InlinePiece>,
        attrs: Vec<AttrEntry>,
        span: Span,
    },
    Heading {
        level: u8,
        title: Vec<InlinePiece>,
        attrs: Vec<AttrEntry>,
        span: Span,
    },
    CodeBlock {
        lang: String,
        text: String,
        attrs: Vec<AttrEntry>,
        span: Span,
    },
    List {
        ordered: bool,
        start: u64,
        items: Vec<Vec<BlockPiece>>,
        attrs: Vec<AttrEntry>,
        span: Span,
    },
    Quote {
        body: Vec<BlockPiece>,
        attrs: Vec<AttrEntry>,
        span: Span,
    },
    Divider {
        attrs: Vec<AttrEntry>,
        span: Span,
    },
    Table {
        headers: Vec<Vec<InlinePiece>>,
        alignments: Vec<TableAlign>,
        rows: Vec<Vec<Vec<InlinePiece>>>,
        attrs: Vec<AttrEntry>,
        span: Span,
    },
    Math {
        text: String,
        attrs: Vec<AttrEntry>,
        span: Span,
    },
    Expr {
        expr: Box<Expr>,
        attrs: Vec<AttrEntry>,
        span: Span,
    },
}

#[derive(Debug)]
pub enum ExprKind {
    Unit,
    /// Typst-style `()`: empty `List<T>` when `List<T>` is expected, or `Unit` otherwise.
    EmptyParen,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    InlineLiteral(Vec<InlinePiece>),
    BlockLiteral(Vec<BlockPiece>),
    ListLiteral(Vec<Expr>),
    For {
        binding: String,
        binding_span: Span,
        iterable: Box<Expr>,
        body: Box<Expr>,
    },
    AttachAttrs(Vec<AttrEntry>, Box<Expr>),
    Name(String),
    Member(Box<Expr>, String),
    Instantiate(Box<Expr>, Vec<Type>),
    Let(String, Box<Expr>, Box<Expr>),
    Seq(Box<Expr>, Box<Expr>),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    TypeMatch {
        param: String,
        param_span: Span,
        arms: Vec<TypeMatchArm>,
    },
    CompileError(String),
    Call(Box<Expr>, Vec<Expr>),
    MethodCall(Box<Expr>, String, Vec<Expr>),
}
