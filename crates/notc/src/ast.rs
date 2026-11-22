//! Source-level declarations and expressions, before name or type resolution.

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
    /// `import "./math.notc";` — alias derived from file stem.
    Default,
    /// `import "./math.notc" as m;`
    Alias { alias: String, span: Span },
    /// `import "./math.notc": inc, add as sum;`
    Members(Vec<ImportMember>),
    /// `import "./math.notc": *;`
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
    Function(Vec<Type>, Box<Type>),
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
    pub parameters: Vec<Param>,
    pub returns: Type,
    pub body: Expr,
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
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub const fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug)]
pub enum ExprKind {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Name(String),
    Member(Box<Expr>, String),
    Let(String, Box<Expr>, Box<Expr>),
    Seq(Box<Expr>, Box<Expr>),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    MethodCall(Box<Expr>, String, Vec<Expr>),
}
