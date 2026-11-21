//! Source-level declarations and expressions, before name or type resolution.

#[derive(Debug)]
pub struct Program {
    /// The only module until imports are supported.
    pub root: Module,
}

#[derive(Debug)]
pub struct Module {
    pub declarations: Vec<Decl>,
}

#[derive(Debug)]
pub enum Decl {
    Function(Function),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Int,
    Bool,
    Function(Vec<Type>, Box<Type>),
}

#[derive(Debug)]
pub struct Function {
    pub name: String,
    pub parameters: Vec<(String, Type)>,
    pub returns: Type,
    pub body: Expr,
}

#[derive(Debug)]
pub enum Expr {
    Int(i64),
    Bool(bool),
    Name(String),
    Let(String, Box<Expr>, Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
}
