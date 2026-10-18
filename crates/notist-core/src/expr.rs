use rowan::TextRange;

use crate::item::{Ctor, Dict, Value};

/// How a call's `[...]` body was written. Frontend-generated calls without
/// an explicit body use `None`; their content contracts are still checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyFlavor {
    None,
    Inline,
    Block,
}

/// An argument before signature binding. Both forms carry expressions;
/// argument order and repeated names remain available for diagnostics.
#[derive(Debug, Clone, PartialEq)]
pub enum CallArg {
    Positional(Expr),
    Named { name: String, value: Expr },
}

impl CallArg {
    pub fn value(&self) -> &Expr {
        match self {
            Self::Positional(value) | Self::Named { value, .. } => value,
        }
    }

    pub fn value_mut(&mut self) -> &mut Expr {
        match self {
            Self::Positional(value) | Self::Named { value, .. } => value,
        }
    }
}

/// IR₁: frontend-independent expressions. A reference reads a binding;
/// only `Call` invokes its callee. Markup sugar lowers to calls whose
/// callees are references, without retaining source markers such as `#`.
///
/// Annotations are carried beside the forest until shaping ends; module
/// annotations are carried by `FrontendOutput` instead.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Literal(Value, TextRange),
    /// A source-delimited fragment in a block content sequence. A soft
    /// newline stays inside this group. Resolve checks the children's public
    /// categories: all inline becomes one paragraph; a sole block passes
    /// through. This is syntax grouping, never a function invocation.
    Fragment {
        children: Vec<Expr>,
        span: TextRange,
    },
    Ref {
        name: String,
        span: TextRange,
    },
    Content {
        forest: Vec<Expr>,
        annotations: Vec<crate::annotation::Annotation>,
        body: BodyFlavor,
        span: TextRange,
    },
    Block {
        bindings: Vec<(String, Expr)>,
        value: Box<Expr>,
        span: TextRange,
    },
    If {
        condition: Box<Expr>,
        yes: Box<Expr>,
        no: Box<Expr>,
        span: TextRange,
    },
    Match {
        subject: Box<Expr>,
        arms: Vec<(Expr, Expr)>,
        span: TextRange,
    },
    Binary {
        op: String,
        left: Box<Expr>,
        right: Box<Expr>,
        span: TextRange,
    },
    Unary {
        op: String,
        value: Box<Expr>,
        span: TextRange,
    },
    Comptime {
        value: Box<Expr>,
        span: TextRange,
    },
    Array(Vec<Expr>, TextRange),
    Dict(Vec<(String, Expr)>, TextRange),
    Call {
        callee: Box<Expr>,
        args: Vec<CallArg>,
        comptime: Vec<CallArg>,
        children: Vec<Expr>,
        body: BodyFlavor,
        attrs: Dict,
        span: TextRange,
    },
}

impl Expr {
    pub fn fragment(children: Vec<Expr>, span: TextRange) -> Self {
        Self::Fragment { children, span }
    }

    pub fn reference(name: impl Into<String>, span: TextRange) -> Self {
        Self::Ref {
            name: name.into(),
            span,
        }
    }

    pub fn call(name: &str, span: TextRange) -> Self {
        Self::Call {
            callee: Box::new(Self::reference(name, span)),
            args: Vec::new(),
            comptime: Vec::new(),
            children: Vec::new(),
            body: BodyFlavor::None,
            attrs: Dict::default(),
            span,
        }
    }

    pub fn text(text: String, span: TextRange) -> Self {
        Self::call("notist::text", span).with_literal_arg("text", Value::Str(text))
    }

    pub fn with_named_arg(mut self, name: impl Into<String>, value: Expr) -> Self {
        let Self::Call { args, .. } = &mut self else {
            panic!("arguments require a call expression");
        };
        args.push(CallArg::Named {
            name: name.into(),
            value,
        });
        self
    }

    /// Convenience for syntax sugar whose argument value is already known.
    pub fn with_literal_arg(self, name: impl Into<String>, value: Value) -> Self {
        let span = self.span();
        self.with_named_arg(name, Self::Literal(value, span))
    }

    pub fn with_comptime(mut self, name: impl Into<String>, value: Expr) -> Self {
        let Self::Call { comptime, .. } = &mut self else {
            panic!("comptime arguments require a call")
        };
        comptime.push(CallArg::Named {
            name: name.into(),
            value,
        });
        self
    }
    pub fn with_category(self, ty: crate::definitions::Type) -> Self {
        let span = self.span();
        self.with_comptime("C", Self::reference(ty.to_string(), span))
    }
    pub fn with_children(mut self, children: Vec<Expr>) -> Self {
        let Self::Call { children: c, .. } = &mut self else {
            panic!("children require a call expression");
        };
        *c = children;
        self
    }

    pub fn span(&self) -> TextRange {
        match self {
            Self::Literal(_, span)
            | Self::Fragment { span, .. }
            | Self::Ref { span, .. }
            | Self::Call { span, .. }
            | Self::Content { span, .. }
            | Self::Block { span, .. }
            | Self::If { span, .. }
            | Self::Match { span, .. }
            | Self::Binary { span, .. }
            | Self::Unary { span, .. }
            | Self::Comptime { span, .. }
            | Self::Array(_, span)
            | Self::Dict(_, span) => *span,
        }
    }

    /// Visit every expression's source range, including callees and arguments.
    /// Frontends can restore offsets after parsing nested or stripped source.
    pub fn map_spans(&mut self, map: &mut impl FnMut(TextRange) -> TextRange) {
        match self {
            Self::Literal(_, span) | Self::Ref { span, .. } => *span = map(*span),
            Self::Fragment { children, span } => {
                *span = map(*span);
                for child in children {
                    child.map_spans(map);
                }
            }
            Self::Call {
                callee,
                args,
                comptime,
                children,
                span,
                ..
            } => {
                *span = map(*span);
                callee.map_spans(map);
                for arg in args.iter_mut().chain(comptime) {
                    arg.value_mut().map_spans(map);
                }
                for child in children {
                    child.map_spans(map);
                }
            }
            Self::Content {
                forest,
                annotations,
                span,
                ..
            } => {
                *span = map(*span);
                for e in forest {
                    e.map_spans(map);
                }
                for annotation in annotations {
                    annotation.map_spans(map);
                }
            }
            Self::Block {
                bindings,
                value,
                span,
            } => {
                *span = map(*span);
                for (_, e) in bindings {
                    e.map_spans(map);
                }
                value.map_spans(map);
            }
            Self::If {
                condition,
                yes,
                no,
                span,
            } => {
                *span = map(*span);
                condition.map_spans(map);
                yes.map_spans(map);
                no.map_spans(map);
            }
            Self::Match {
                subject,
                arms,
                span,
            } => {
                *span = map(*span);
                subject.map_spans(map);
                for (a, b) in arms {
                    a.map_spans(map);
                    b.map_spans(map);
                }
            }
            Self::Binary {
                left, right, span, ..
            } => {
                *span = map(*span);
                left.map_spans(map);
                right.map_spans(map);
            }
            Self::Unary { value, span, .. } | Self::Comptime { value, span } => {
                *span = map(*span);
                value.map_spans(map);
            }
            Self::Array(values, span) => {
                *span = map(*span);
                for e in values {
                    e.map_spans(map);
                }
            }
            Self::Dict(values, span) => {
                *span = map(*span);
                for (_, e) in values {
                    e.map_spans(map);
                }
            }
        }
    }

    /// Attach metadata to a content call. A reference has no node of its own;
    /// callers must diagnose or otherwise handle an invalid annotation target.
    pub fn set_attrs(&mut self, new_attrs: Dict) -> Result<(), Dict> {
        if let Expr::Call { attrs, .. } = self {
            *attrs = new_attrs;
            Ok(())
        } else {
            Err(new_attrs)
        }
    }

    pub fn literal_value(&self) -> Option<&Value> {
        match self {
            Self::Literal(value, _) => Some(value),
            _ => None,
        }
    }

    pub fn named_arg(&self, name: &str) -> Option<&Expr> {
        let Self::Call { args, .. } = self else {
            return None;
        };
        args.iter().rev().find_map(|arg| match arg {
            CallArg::Named { name: key, value } if key == name => Some(value),
            _ => None,
        })
    }

    pub fn callee_name(&self) -> Option<&str> {
        match self {
            Self::Call { callee, .. } => match callee.as_ref() {
                Self::Ref { name, .. } => Some(name),
                _ => None,
            },
            _ => None,
        }
    }
}

/// IR₂: a resolved content node with bound data fields and checked pending
/// computations. Shape carries those computations opaquely and uses only the
/// public content contract. IR₂′ uses the same representation after Shape has
/// finalized section structure. Resolve has already validated source
/// fragments and constructed their paragraphs.
#[derive(Debug, Clone, PartialEq)]
pub struct RExpr {
    pub ctor: Ctor,
    pub level: Option<crate::builtins::Level>,
    pub instance: Option<crate::definitions::FunctionInstance>,
    /// Checked computation retained through Shape and copied into Item.
    pub pending: Option<Box<crate::evaluation::PendingEvaluation>>,
    pub fields: Dict,
    pub children: Vec<RExpr>,
    pub body: BodyFlavor,
    pub attrs: Dict,
    pub span: TextRange,
}

impl RExpr {
    pub fn new(ctor: Ctor, span: TextRange) -> Self {
        Self {
            ctor,
            level: None,
            instance: None,
            pending: None,
            fields: Dict::default(),
            children: Vec::new(),
            body: BodyFlavor::None,
            attrs: Dict::default(),
            span,
        }
    }

    pub fn with_field(mut self, key: impl Into<String>, value: Value) -> Self {
        self.fields.insert(key, value);
        self
    }

    pub fn with_children(mut self, children: Vec<Self>) -> Self {
        self.children = children;
        self
    }

    pub fn span(&self) -> TextRange {
        self.span
    }
}
