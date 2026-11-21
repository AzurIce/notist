use crate::{Decl, Expr, ExprKind, Function, Method, Program, Span, Type};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FunctionId(pub(crate) usize);

#[derive(Debug)]
pub struct CheckedProgram {
    /// Top-level entry lookup, not a table of lexical bindings.
    pub(crate) function_names: HashMap<String, FunctionId>,
    pub(crate) functions: Vec<CheckedFunction>,
}

#[derive(Debug)]
pub(crate) struct CheckedFunction {
    pub(crate) arity: usize,
    pub(crate) body: CheckedExpr,
}

#[derive(Debug)]
pub(crate) enum CheckedExpr {
    Int(i64),
    Bool(bool),
    Local(usize),
    Function(FunctionId),
    Let(Box<Self>, Box<Self>),
    Add(Box<Self>, Box<Self>),
    If(Box<Self>, Box<Self>, Box<Self>),
    Call(Box<Self>, Vec<Self>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub declaration: String,
    pub span: Span,
    pub error: TypeError,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TypeError {
    DuplicateFunction,
    DuplicateMethod,
    DuplicateParameter(String),
    UnknownName(String),
    UnknownMethod { receiver: Type, method: String },
    NotCallable(Type),
    Arity { expected: usize, actual: usize },
    Mismatch { expected: Type, actual: Type },
}

struct MethodEntry {
    id: FunctionId,
    parameters: Vec<Type>,
    returns: Type,
}

/// Collect all signatures first, then check every body, independent of reachability.
/// The prototype reports at most one body error per definition.
pub fn check(program: &Program) -> Result<CheckedProgram, Vec<Diagnostic>> {
    let declarations = &program.root.declarations;
    let mut function_names = HashMap::new();
    let mut methods = HashMap::new();
    let mut signatures = Vec::with_capacity(declarations.len());
    let mut diagnostics = Vec::new();

    for (index, decl) in declarations.iter().enumerate() {
        let id = FunctionId(index);
        match decl {
            Decl::Function(function) => {
                if function_names.insert(function.name.clone(), id).is_some() {
                    diagnostics.push(Diagnostic {
                        declaration: function.name.clone(),
                        span: function.name_span,
                        error: TypeError::DuplicateFunction,
                    });
                }
                signatures.push(Type::Function(
                    function.parameters.iter().map(|p| p.ty.clone()).collect(),
                    Box::new(function.returns.clone()),
                ));
            }
            Decl::Method(method) => {
                let key = (method.receiver_type.clone(), method.name.clone());
                let parameters: Vec<Type> =
                    method.parameters.iter().map(|p| p.ty.clone()).collect();
                if methods
                    .insert(
                        key,
                        MethodEntry {
                            id,
                            parameters: parameters.clone(),
                            returns: method.returns.clone(),
                        },
                    )
                    .is_some()
                {
                    diagnostics.push(Diagnostic {
                        declaration: method_declaration_name(method),
                        span: method.name_span,
                        error: TypeError::DuplicateMethod,
                    });
                }
                let mut full_params = Vec::with_capacity(1 + parameters.len());
                full_params.push(method.receiver_type.clone());
                full_params.extend(parameters);
                signatures.push(Type::Function(full_params, Box::new(method.returns.clone())));
            }
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let mut checked = Vec::with_capacity(declarations.len());
    for decl in declarations {
        let mut checker = Checker {
            function_names: &function_names,
            methods: &methods,
            signatures: &signatures,
            locals: HashMap::new(),
            depth: 0,
        };
        let (declaration, result) = match decl {
            Decl::Function(function) => (function.name.clone(), checker.function(function)),
            Decl::Method(method) => (method_declaration_name(method), checker.method(method)),
        };
        match result {
            Ok(function) => checked.push(function),
            Err((span, error)) => diagnostics.push(Diagnostic {
                declaration,
                span,
                error,
            }),
        }
    }
    if diagnostics.is_empty() {
        Ok(CheckedProgram {
            function_names,
            functions: checked,
        })
    } else {
        Err(diagnostics)
    }
}

fn method_declaration_name(method: &Method) -> String {
    format!("{}.{}", format_type(&method.receiver_type), method.name)
}

fn format_type(ty: &Type) -> String {
    match ty {
        Type::Int => "Int".into(),
        Type::Bool => "Bool".into(),
        Type::Function(parameters, returns) => {
            let params = parameters
                .iter()
                .map(format_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("fn({params}) -> {}", format_type(returns))
        }
    }
}

struct Checker<'a> {
    function_names: &'a HashMap<String, FunctionId>,
    methods: &'a HashMap<(Type, String), MethodEntry>,
    signatures: &'a [Type],
    locals: HashMap<&'a str, (usize, Type)>,
    depth: usize,
}

impl<'a> Checker<'a> {
    fn function(&mut self, function: &'a Function) -> Result<CheckedFunction, (Span, TypeError)> {
        for param in &function.parameters {
            if self
                .locals
                .insert(param.name.as_str(), (self.depth, param.ty.clone()))
                .is_some()
            {
                return Err((param.span, TypeError::DuplicateParameter(param.name.clone())));
            }
            self.depth += 1;
        }
        let (body, ty) = self.expression(&function.body)?;
        expect(&function.returns, ty, function.body.span)?;
        Ok(CheckedFunction {
            arity: function.parameters.len(),
            body,
        })
    }

    fn method(&mut self, method: &'a Method) -> Result<CheckedFunction, (Span, TypeError)> {
        self.locals.insert(
            method.receiver_name.as_str(),
            (self.depth, method.receiver_type.clone()),
        );
        self.depth += 1;
        for param in &method.parameters {
            if self
                .locals
                .insert(param.name.as_str(), (self.depth, param.ty.clone()))
                .is_some()
            {
                return Err((param.span, TypeError::DuplicateParameter(param.name.clone())));
            }
            self.depth += 1;
        }
        let (body, ty) = self.expression(&method.body)?;
        expect(&method.returns, ty, method.body.span)?;
        Ok(CheckedFunction {
            arity: 1 + method.parameters.len(),
            body,
        })
    }

    fn expression(&mut self, expr: &'a Expr) -> Result<(CheckedExpr, Type), (Span, TypeError)> {
        use CheckedExpr as C;
        Ok(match &expr.kind {
            ExprKind::Int(n) => (C::Int(*n), Type::Int),
            ExprKind::Bool(b) => (C::Bool(*b), Type::Bool),
            ExprKind::Name(name) => {
                if let Some((slot, ty)) = self.locals.get(name.as_str()) {
                    (C::Local(*slot), ty.clone())
                } else if let Some(&id) = self.function_names.get(name) {
                    (C::Function(id), self.signatures[id.0].clone())
                } else {
                    return Err((expr.span, TypeError::UnknownName(name.clone())));
                }
            }
            ExprKind::Let(name, initializer, body) => {
                let (initializer, ty) = self.expression(initializer)?;
                let previous = self.locals.insert(name, (self.depth, ty));
                self.depth += 1;
                let result = self.expression(body);
                self.depth -= 1;
                if let Some(previous) = previous {
                    self.locals.insert(name, previous);
                } else {
                    self.locals.remove(name.as_str());
                }
                let (body, ty) = result?;
                (C::Let(Box::new(initializer), Box::new(body)), ty)
            }
            ExprKind::Add(left, right) => (
                C::Add(
                    Box::new(self.expect_expr(left, &Type::Int)?),
                    Box::new(self.expect_expr(right, &Type::Int)?),
                ),
                Type::Int,
            ),
            ExprKind::If(condition, yes, no) => {
                let condition = self.expect_expr(condition, &Type::Bool)?;
                let (yes, ty) = self.expression(yes)?;
                let no = self.expect_expr(no, &ty)?;
                (C::If(Box::new(condition), Box::new(yes), Box::new(no)), ty)
            }
            ExprKind::Call(callee, arguments) => {
                let (checked_callee, ty) = self.expression(callee)?;
                let Type::Function(parameters, returns) = ty else {
                    return Err((callee.span, TypeError::NotCallable(ty)));
                };
                if parameters.len() != arguments.len() {
                    return Err((
                        expr.span,
                        TypeError::Arity {
                            expected: parameters.len(),
                            actual: arguments.len(),
                        },
                    ));
                }
                let arguments = arguments
                    .iter()
                    .zip(&parameters)
                    .map(|(arg, ty)| self.expect_expr(arg, ty))
                    .collect::<Result<_, _>>()?;
                (C::Call(Box::new(checked_callee), arguments), *returns)
            }
            ExprKind::MethodCall(receiver, method_name, arguments) => {
                let (receiver_expr, receiver_ty) = self.expression(receiver)?;
                let Some(entry) = self.methods.get(&(receiver_ty.clone(), method_name.clone()))
                else {
                    return Err((
                        expr.span,
                        TypeError::UnknownMethod {
                            receiver: receiver_ty,
                            method: method_name.clone(),
                        },
                    ));
                };
                if entry.parameters.len() != arguments.len() {
                    return Err((
                        expr.span,
                        TypeError::Arity {
                            expected: entry.parameters.len(),
                            actual: arguments.len(),
                        },
                    ));
                }
                let mut call_args = Vec::with_capacity(1 + arguments.len());
                call_args.push(receiver_expr);
                for (arg, ty) in arguments.iter().zip(&entry.parameters) {
                    call_args.push(self.expect_expr(arg, ty)?);
                }
                (
                    C::Call(Box::new(C::Function(entry.id)), call_args),
                    entry.returns.clone(),
                )
            }
        })
    }

    fn expect_expr(
        &mut self,
        expr: &'a Expr,
        expected: &Type,
    ) -> Result<CheckedExpr, (Span, TypeError)> {
        let (checked, actual) = self.expression(expr)?;
        expect(expected, actual, expr.span)?;
        Ok(checked)
    }
}

fn expect(expected: &Type, actual: Type, span: Span) -> Result<(), (Span, TypeError)> {
    if *expected == actual {
        Ok(())
    } else {
        Err((
            span,
            TypeError::Mismatch {
                expected: expected.clone(),
                actual,
            },
        ))
    }
}
