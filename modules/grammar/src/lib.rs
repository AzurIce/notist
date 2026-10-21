//! Parse the documentation grammar notation and render SVG railroad diagrams.
//!
//! This library describes grammar; it does not execute it or generate a parser.
//! Match and construction structures share one notation; computations are
//! retained as annotations, never evaluated by this library.

mod parser;
mod render;

pub use parser::parse;
pub use render::{RenderOptions, Theme, render};

use std::{fmt, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grammar {
    pub rules: Vec<Rule>,
    pub imports: Vec<Import>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Lex,
    Syntax,
    Ast,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub layer: Layer,
    pub name: String,
    pub is_root: bool,
    /// Parameter declarations are preserved verbatim, without evaluation.
    pub parameters: Option<String>,
    pub expression: Expr,
    /// Optional construction after a successful match. Does not consume input.
    pub result: Option<Expr>,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub kind: String,
    pub name: String,
    pub parameters: Option<String>,
    /// A documentation locator, not a request to read a file.
    pub from: String,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    Literal(String),
    Unicode {
        character: char,
        hex: String,
    },
    Prose(String),
    Complement(Box<Expr>),
    Annotated {
        expression: Box<Expr>,
        suffix: Option<String>,
        footnote: Option<String>,
    },
    Reference {
        name: String,
        arguments: Option<String>,
    },
    /// A node with an explicitly described child structure.
    Node {
        name: String,
        arguments: Option<String>,
        children: Box<Expr>,
    },
    /// A balanced, opaque pure computation, used only in results.
    Computation(String),
    CharacterSet(Vec<CharacterSetItem>),
    Sequence(Vec<Expr>),
    OrderedChoice(Vec<Expr>),
    Repeat {
        expression: Box<Expr>,
        min: String,
        max: Option<String>,
        limit: RangeLimit,
        count: Option<String>,
    },
    Lookahead {
        positive: bool,
        expression: Box<Expr>,
    },
    Capture {
        name: String,
        expression: Box<Expr>,
    },
    Predicate(String),
    Binding {
        name: String,
        value: String,
    },
    Within {
        end: String,
        expression: Box<Expr>,
    },
    Scanner {
        probe: bool,
        name: String,
        arguments: Option<String>,
    },
    Commit,
    Empty,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CharacterSetItem {
    Character(char),
    Range(char, char),
    Reference(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeLimit {
    HalfOpen,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// UTF-8 byte range in the original source.
    pub span: Range<usize>,
    /// One-based line and Unicode scalar column.
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl Error {
    pub(crate) fn new(source: &str, span: Range<usize>, message: impl Into<String>) -> Self {
        let before = &source[..span.start];
        let mut line = 1;
        let mut column = 1;
        let mut chars = before.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    line += 1;
                    column = 1;
                }
                '\n' => {
                    line += 1;
                    column = 1;
                }
                _ => column += 1,
            }
        }
        Self {
            line,
            column,
            span,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for Error {}

/// Convenience entry point shared by native callers and the browser component.
pub fn render_source(source: &str, options: &RenderOptions) -> Result<String, Error> {
    render(&parse(source)?, options).map_err(|message| Error::new(source, 0..0, message))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn render_grammar(
    source: &str,
    rule: &str,
    theme: &str,
) -> Result<String, wasm_bindgen::JsValue> {
    let theme = theme
        .parse()
        .map_err(|message: String| wasm_bindgen::JsValue::from_str(&message))?;
    render_source(
        source,
        &RenderOptions {
            rule: (!rule.is_empty()).then(|| rule.to_owned()),
            theme,
            ..RenderOptions::default()
        },
    )
    .map_err(|error| wasm_bindgen::JsValue::from_str(&error.to_string()))
}
