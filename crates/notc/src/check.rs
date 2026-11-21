use crate::{Decl, Expr, Function, Program, Type};
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
    pub function: String,
    pub error: TypeError,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TypeError {
    DuplicateFunction,
    DuplicateParameter(String),
    UnknownName(String),
    NotCallable(Type),
    Arity { expected: usize, actual: usize },
    Mismatch { expected: Type, actual: Type },
}

/// Collect all signatures first, then check every body, independent of reachability.
/// The prototype reports at most one body error per definition.
pub fn check(program: &Program) -> Result<CheckedProgram, Vec<Diagnostic>> {
    let declarations = &program.root.declarations;
    let mut function_names = HashMap::with_capacity(declarations.len());
    let mut signatures = Vec::with_capacity(declarations.len());
    let mut diagnostics = Vec::new();
    for (index, Decl::Function(function)) in declarations.iter().enumerate() {
        if function_names
            .insert(function.name.clone(), FunctionId(index))
            .is_some()
        {
            diagnostics.push(Diagnostic {
                function: function.name.clone(),
                error: TypeError::DuplicateFunction,
            });
        }
        signatures.push(Type::Function(
            function
                .parameters
                .iter()
                .map(|(_, ty)| ty.clone())
                .collect(),
            Box::new(function.returns.clone()),
        ));
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let mut checked = Vec::with_capacity(declarations.len());
    for Decl::Function(function) in declarations {
        let mut checker = Checker {
            function_names: &function_names,
            signatures: &signatures,
            locals: HashMap::new(),
            depth: 0,
        };
        match checker.function(function) {
            Ok(function) => checked.push(function),
            Err(error) => diagnostics.push(Diagnostic {
                function: function.name.clone(),
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

struct Checker<'a> {
    function_names: &'a HashMap<String, FunctionId>,
    signatures: &'a [Type],
    locals: HashMap<&'a str, (usize, Type)>,
    depth: usize,
}

impl<'a> Checker<'a> {
    fn function(&mut self, function: &'a Function) -> Result<CheckedFunction, TypeError> {
        for (name, ty) in &function.parameters {
            if self.locals.insert(name, (self.depth, ty.clone())).is_some() {
                return Err(TypeError::DuplicateParameter(name.clone()));
            }
            self.depth += 1;
        }
        let (body, ty) = self.expression(&function.body)?;
        expect(&function.returns, ty)?;
        Ok(CheckedFunction {
            arity: function.parameters.len(),
            body,
        })
    }

    fn expression(&mut self, expr: &'a Expr) -> Result<(CheckedExpr, Type), TypeError> {
        use CheckedExpr as C;
        Ok(match expr {
            Expr::Int(n) => (C::Int(*n), Type::Int),
            Expr::Bool(b) => (C::Bool(*b), Type::Bool),
            Expr::Name(name) => {
                if let Some((slot, ty)) = self.locals.get(name.as_str()) {
                    (C::Local(*slot), ty.clone())
                } else if let Some(&id) = self.function_names.get(name) {
                    (C::Function(id), self.signatures[id.0].clone())
                } else {
                    return Err(TypeError::UnknownName(name.clone()));
                }
            }
            Expr::Let(name, initializer, body) => {
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
            Expr::Add(left, right) => (
                C::Add(
                    Box::new(self.expect_expr(left, &Type::Int)?),
                    Box::new(self.expect_expr(right, &Type::Int)?),
                ),
                Type::Int,
            ),
            Expr::If(condition, yes, no) => {
                let condition = self.expect_expr(condition, &Type::Bool)?;
                let (yes, ty) = self.expression(yes)?;
                let no = self.expect_expr(no, &ty)?;
                (C::If(Box::new(condition), Box::new(yes), Box::new(no)), ty)
            }
            Expr::Call(callee, arguments) => {
                let (callee, ty) = self.expression(callee)?;
                let Type::Function(parameters, returns) = ty else {
                    return Err(TypeError::NotCallable(ty));
                };
                if parameters.len() != arguments.len() {
                    return Err(TypeError::Arity {
                        expected: parameters.len(),
                        actual: arguments.len(),
                    });
                }
                let arguments = arguments
                    .iter()
                    .zip(&parameters)
                    .map(|(expr, ty)| self.expect_expr(expr, ty))
                    .collect::<Result<_, _>>()?;
                (C::Call(Box::new(callee), arguments), *returns)
            }
        })
    }

    fn expect_expr(&mut self, expr: &'a Expr, expected: &Type) -> Result<CheckedExpr, TypeError> {
        let (expr, actual) = self.expression(expr)?;
        expect(expected, actual)?;
        Ok(expr)
    }
}

fn expect(expected: &Type, actual: Type) -> Result<(), TypeError> {
    if *expected == actual {
        Ok(())
    } else {
        Err(TypeError::Mismatch {
            expected: expected.clone(),
            actual,
        })
    }
}
