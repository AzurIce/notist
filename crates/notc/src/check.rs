use crate::{BinaryOp, Decl, Expr, ExprKind, Function, Method, Module, Program, Span, Type, UnaryOp};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ModuleId(pub(crate) usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DeclId {
    pub(crate) module: ModuleId,
    pub(crate) index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FunctionId(pub(crate) usize);

#[derive(Debug)]
pub struct CheckedProgram {
    /// Top-level entry lookup, not a table of lexical bindings.
    pub(crate) function_names: HashMap<String, FunctionId>,
    pub(crate) functions: Vec<CheckedFunction>,
}

impl CheckedProgram {
    /// Number of lowered functions/methods present in this executable program.
    pub fn function_count(&self) -> usize {
        self.functions.len()
    }
}

#[derive(Debug)]
pub(crate) struct CheckedFunction {
    pub(crate) arity: usize,
    pub(crate) body: CheckedExpr,
}

#[derive(Debug)]
pub(crate) enum CheckedExpr {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Arc<str>),
    Local(usize),
    Function(FunctionId),
    Let(Box<Self>, Box<Self>),
    Seq(Box<Self>, Box<Self>),
    Unary(UnaryOp, Box<Self>),
    Binary(BinaryOp, Box<Self>, Box<Self>),
    If(Box<Self>, Box<Self>, Box<Self>),
    Call(Box<Self>, Vec<Self>),
}

/// Checked expression before entry-reachability lowering.
/// Uses module-qualified `DeclId` so per-module checking never depends on a
/// global flat function layout.
#[derive(Debug)]
pub(crate) enum ModuleExpr {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Arc<str>),
    Local(usize),
    Function(DeclId),
    Let(Box<Self>, Box<Self>),
    Seq(Box<Self>, Box<Self>),
    Unary(UnaryOp, Box<Self>),
    Binary(BinaryOp, Box<Self>, Box<Self>),
    If(Box<Self>, Box<Self>, Box<Self>),
    Call(Box<Self>, Vec<Self>),
}

#[derive(Debug)]
pub(crate) struct CheckedDecl {
    pub(crate) arity: usize,
    pub(crate) body: ModuleExpr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Binding {
    Function(DeclId),
    Namespace(ModuleId),
}

#[derive(Debug, Clone)]
pub(crate) struct MethodSig {
    pub(crate) id: DeclId,
    pub(crate) parameters: Vec<Type>,
    pub(crate) returns: Type,
}

/// Body-independent lexical scope for checking a single module.
pub(crate) struct ModuleScope {
    pub(crate) bindings: HashMap<String, Binding>,
    pub(crate) methods: HashMap<(Type, String), MethodSig>,
}

/// Project-wide signature & export view read by per-module body checking.
/// Contains no function bodies, keeping body checking isolated per module.
pub(crate) struct InterfaceView<'a> {
    pub(crate) module_paths: &'a [String],
    pub(crate) exports: &'a [HashMap<String, Binding>],
    pub(crate) signatures: &'a [Vec<Type>],
}

impl InterfaceView<'_> {
    fn signature(&self, id: DeclId) -> &Type {
        &self.signatures[id.module.0][id.index]
    }
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
    DuplicateBinding(String),
    DuplicateParameter(String),
    InvalidImportPath(String),
    InvalidImportAlias(String),
    MissingModule(String),
    ImportCycle(Vec<String>),
    UnknownExport { module: String, name: String },
    AmbiguousMethod { receiver: Type, method: String },
    NamespaceAsValue(String),
    NotNamespace(Type),
    UnknownName(String),
    UnknownMethod { receiver: Type, method: String },
    UnsupportedUnaryOp { op: UnaryOp, operand: Type },
    UnsupportedBinaryOp {
        op: BinaryOp,
        left: Type,
        right: Type,
    },
    NotCallable(Type),
    Arity { expected: usize, actual: usize },
    Mismatch { expected: Type, actual: Type },
}

/// Body-independent signature summary extracted from a single parsed module.
pub(crate) struct ModuleInterface {
    pub(crate) signatures: Vec<Type>,
    pub(crate) bindings: HashMap<String, Binding>,
    pub(crate) methods: HashMap<(Type, String), MethodSig>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

/// Collect all signatures first, then check every body, independent of reachability.
/// Single-file convenience entry point; rejects `import` declarations.
pub fn check(program: &Program) -> Result<CheckedProgram, Vec<Diagnostic>> {
    let module_id = ModuleId(0);
    let interface = collect_module_interface(module_id, &program.root, false);
    let mut diagnostics = interface.diagnostics;
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let exports = vec![interface.bindings.clone()];
    let signatures = vec![interface.signatures];
    let paths = vec!["<root>".to_owned()];
    let scope = ModuleScope {
        bindings: interface.bindings,
        methods: interface.methods,
    };
    let view = InterfaceView {
        module_paths: &paths,
        exports: &exports,
        signatures: &signatures,
    };

    let (checked_decls, body_diags) = check_module_bodies(&program.root, &scope, &view);
    diagnostics.extend(body_diags);
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let mut function_names = HashMap::new();
    for (name, binding) in &scope.bindings {
        if let Binding::Function(decl_id) = binding {
            function_names.insert(name.clone(), FunctionId(decl_id.index));
        }
    }
    let functions = checked_decls
        .into_iter()
        .map(|decl| CheckedFunction {
            arity: decl.arity,
            body: lower_single_module_expr(decl.body),
        })
        .collect();

    Ok(CheckedProgram {
        function_names,
        functions,
    })
}

fn lower_single_module_expr(expr: ModuleExpr) -> CheckedExpr {
    use CheckedExpr as C;
    use ModuleExpr as M;
    match expr {
        M::Unit => C::Unit,
        M::Bool(b) => C::Bool(b),
        M::Int(n) => C::Int(n),
        M::Float(f) => C::Float(f),
        M::String(s) => C::String(s),
        M::Local(slot) => C::Local(slot),
        M::Function(id) => C::Function(FunctionId(id.index)),
        M::Let(init, body) => C::Let(
            Box::new(lower_single_module_expr(*init)),
            Box::new(lower_single_module_expr(*body)),
        ),
        M::Seq(first, second) => C::Seq(
            Box::new(lower_single_module_expr(*first)),
            Box::new(lower_single_module_expr(*second)),
        ),
        M::Unary(op, operand) => C::Unary(op, Box::new(lower_single_module_expr(*operand))),
        M::Binary(op, left, right) => C::Binary(
            op,
            Box::new(lower_single_module_expr(*left)),
            Box::new(lower_single_module_expr(*right)),
        ),
        M::If(cond, yes, no) => C::If(
            Box::new(lower_single_module_expr(*cond)),
            Box::new(lower_single_module_expr(*yes)),
            Box::new(lower_single_module_expr(*no)),
        ),
        M::Call(callee, args) => C::Call(
            Box::new(lower_single_module_expr(*callee)),
            args.into_iter().map(lower_single_module_expr).collect(),
        ),
    }
}

/// Layer 1 helper: extract signatures, local bindings, and local methods from a
/// single parsed module without inspecting any function or method body.
pub(crate) fn collect_module_interface(
    module_id: ModuleId,
    module: &Module,
    allow_imports: bool,
) -> ModuleInterface {
    let mut signatures = Vec::new();
    let mut bindings = HashMap::new();
    let mut methods = HashMap::new();
    let mut diagnostics = Vec::new();

    for decl in &module.declarations {
        match decl {
            Decl::Import(import) => {
                if !allow_imports {
                    diagnostics.push(Diagnostic {
                        declaration: format!("import \"{}\"", import.path),
                        span: import.path_span,
                        error: TypeError::MissingModule(import.path.clone()),
                    });
                }
            }
            Decl::Function(function) => {
                let id = DeclId {
                    module: module_id,
                    index: signatures.len(),
                };
                if bindings
                    .insert(function.name.clone(), Binding::Function(id))
                    .is_some()
                {
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
                let id = DeclId {
                    module: module_id,
                    index: signatures.len(),
                };
                let key = (method.receiver_type.clone(), method.name.clone());
                let parameters: Vec<Type> =
                    method.parameters.iter().map(|p| p.ty.clone()).collect();
                if methods
                    .insert(
                        key,
                        MethodSig {
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

    ModuleInterface {
        signatures,
        bindings,
        methods,
        diagnostics,
    }
}

/// Layer 2: check every function and method body in a single module using only
/// its resolved `ModuleScope` and project `InterfaceView` (never reading other
/// modules' bodies).
pub(crate) fn check_module_bodies(
    module: &Module,
    scope: &ModuleScope,
    view: &InterfaceView<'_>,
) -> (Vec<CheckedDecl>, Vec<Diagnostic>) {
    let mut checked = Vec::new();
    let mut diagnostics = Vec::new();

    for decl in &module.declarations {
        let mut checker = Checker {
            scope,
            view,
            locals: HashMap::new(),
            depth: 0,
        };
        let (declaration, result) = match decl {
            Decl::Import(_) => continue,
            Decl::Function(function) => (function.name.clone(), checker.function(function)),
            Decl::Method(method) => (method_declaration_name(method), checker.method(method)),
        };
        match result {
            Ok(decl) => checked.push(decl),
            Err((span, error)) => diagnostics.push(Diagnostic {
                declaration,
                span,
                error,
            }),
        }
    }

    (checked, diagnostics)
}

pub(crate) fn method_declaration_name(method: &Method) -> String {
    format!("{}.{}", format_type(&method.receiver_type), method.name)
}

pub(crate) fn format_type(ty: &Type) -> String {
    match ty {
        Type::Unit => "Unit".into(),
        Type::Bool => "Bool".into(),
        Type::Int => "Int".into(),
        Type::Float => "Float".into(),
        Type::String => "String".into(),
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
    scope: &'a ModuleScope,
    view: &'a InterfaceView<'a>,
    locals: HashMap<&'a str, (usize, Type)>,
    depth: usize,
}

impl<'a> Checker<'a> {
    fn function(&mut self, function: &'a Function) -> Result<CheckedDecl, (Span, TypeError)> {
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
        Ok(CheckedDecl {
            arity: function.parameters.len(),
            body,
        })
    }

    fn method(&mut self, method: &'a Method) -> Result<CheckedDecl, (Span, TypeError)> {
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
        Ok(CheckedDecl {
            arity: 1 + method.parameters.len(),
            body,
        })
    }

    /// Resolve an expression as a module namespace chain (e.g. `math` or `a.b`),
    /// returning `Ok(None)` when the expression refers to a runtime value or is
    /// shadowed by a local binding.
    fn resolve_namespace(&self, expr: &Expr) -> Result<Option<ModuleId>, (Span, TypeError)> {
        match &expr.kind {
            ExprKind::Name(name) => {
                if self.locals.contains_key(name.as_str()) {
                    return Ok(None);
                }
                match self.scope.bindings.get(name) {
                    Some(Binding::Namespace(module_id)) => Ok(Some(*module_id)),
                    _ => Ok(None),
                }
            }
            ExprKind::Member(base, member) => {
                let Some(module_id) = self.resolve_namespace(base)? else {
                    return Ok(None);
                };
                match self.view.exports[module_id.0].get(member) {
                    Some(Binding::Namespace(next)) => Ok(Some(*next)),
                    Some(Binding::Function(_)) => Ok(None),
                    None => Err((
                        expr.span,
                        TypeError::UnknownExport {
                            module: self.view.module_paths[module_id.0].clone(),
                            name: member.clone(),
                        },
                    )),
                }
            }
            _ => Ok(None),
        }
    }

    fn expression(&mut self, expr: &'a Expr) -> Result<(ModuleExpr, Type), (Span, TypeError)> {
        use ModuleExpr as M;
        Ok(match &expr.kind {
            ExprKind::Unit => (M::Unit, Type::Unit),
            ExprKind::Bool(b) => (M::Bool(*b), Type::Bool),
            ExprKind::Int(n) => (M::Int(*n), Type::Int),
            ExprKind::Float(f) => (M::Float(*f), Type::Float),
            ExprKind::String(s) => (M::String(Arc::from(s.as_str())), Type::String),
            ExprKind::Name(name) => {
                if let Some((slot, ty)) = self.locals.get(name.as_str()) {
                    (M::Local(*slot), ty.clone())
                } else if let Some(&binding) = self.scope.bindings.get(name) {
                    match binding {
                        Binding::Function(id) => (M::Function(id), self.view.signature(id).clone()),
                        Binding::Namespace(_) => {
                            return Err((expr.span, TypeError::NamespaceAsValue(name.clone())));
                        }
                    }
                } else {
                    return Err((expr.span, TypeError::UnknownName(name.clone())));
                }
            }
            ExprKind::Member(base, member) => {
                if let Some(module_id) = self.resolve_namespace(base)? {
                    match self.view.exports[module_id.0].get(member) {
                        Some(&Binding::Function(id)) => {
                            (M::Function(id), self.view.signature(id).clone())
                        }
                        Some(Binding::Namespace(_)) => {
                            return Err((expr.span, TypeError::NamespaceAsValue(member.clone())));
                        }
                        None => {
                            return Err((
                                expr.span,
                                TypeError::UnknownExport {
                                    module: self.view.module_paths[module_id.0].clone(),
                                    name: member.clone(),
                                },
                            ));
                        }
                    }
                } else {
                    let (_, ty) = self.expression(base)?;
                    return Err((expr.span, TypeError::NotNamespace(ty)));
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
                (M::Let(Box::new(initializer), Box::new(body)), ty)
            }
            ExprKind::Seq(first, second) => {
                let (first, _) = self.expression(first)?;
                let (second, ty) = self.expression(second)?;
                (M::Seq(Box::new(first), Box::new(second)), ty)
            }
            ExprKind::Unary(op, operand) => match op {
                UnaryOp::Not => {
                    let operand = self.expect_expr(operand, &Type::Bool)?;
                    (M::Unary(*op, Box::new(operand)), Type::Bool)
                }
                UnaryOp::Neg => {
                    let (checked, ty) = self.expression(operand)?;
                    match ty {
                        Type::Int | Type::Float => (M::Unary(*op, Box::new(checked)), ty),
                        other => {
                            return Err((
                                expr.span,
                                TypeError::UnsupportedUnaryOp {
                                    op: *op,
                                    operand: other,
                                },
                            ));
                        }
                    }
                }
            },
            ExprKind::Binary(op, left, right) => {
                let (checked_left, left_ty, checked_right, result_ty) =
                    self.check_binary(*op, left, right, expr.span)?;
                let _ = left_ty;
                (
                    M::Binary(*op, Box::new(checked_left), Box::new(checked_right)),
                    result_ty,
                )
            }
            ExprKind::If(condition, yes, no) => {
                let condition = self.expect_expr(condition, &Type::Bool)?;
                let (yes, ty) = self.expression(yes)?;
                let no = self.expect_expr(no, &ty)?;
                (M::If(Box::new(condition), Box::new(yes), Box::new(no)), ty)
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
                (M::Call(Box::new(checked_callee), arguments), *returns)
            }
            ExprKind::MethodCall(receiver, method_name, arguments) => {
                if let Some(module_id) = self.resolve_namespace(receiver)? {
                    match self.view.exports[module_id.0].get(method_name) {
                        Some(&Binding::Function(id)) => {
                            let Type::Function(parameters, returns) =
                                self.view.signature(id).clone()
                            else {
                                unreachable!("function declaration always has function signature");
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
                            let checked_args = arguments
                                .iter()
                                .zip(&parameters)
                                .map(|(arg, ty)| self.expect_expr(arg, ty))
                                .collect::<Result<_, _>>()?;
                            return Ok((M::Call(Box::new(M::Function(id)), checked_args), *returns));
                        }
                        Some(Binding::Namespace(_)) => {
                            return Err((
                                expr.span,
                                TypeError::NamespaceAsValue(method_name.clone()),
                            ));
                        }
                        None => {
                            return Err((
                                expr.span,
                                TypeError::UnknownExport {
                                    module: self.view.module_paths[module_id.0].clone(),
                                    name: method_name.clone(),
                                },
                            ));
                        }
                    }
                }

                let (receiver_expr, receiver_ty) = self.expression(receiver)?;
                let Some(entry) = self
                    .scope
                    .methods
                    .get(&(receiver_ty.clone(), method_name.clone()))
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
                    M::Call(Box::new(M::Function(entry.id)), call_args),
                    entry.returns.clone(),
                )
            }
        })
    }

    fn check_binary(
        &mut self,
        op: BinaryOp,
        left: &'a Expr,
        right: &'a Expr,
        span: Span,
    ) -> Result<(ModuleExpr, Type, ModuleExpr, Type), (Span, TypeError)> {
        match op {
            BinaryOp::Or | BinaryOp::And => {
                let checked_left = self.expect_expr(left, &Type::Bool)?;
                let checked_right = self.expect_expr(right, &Type::Bool)?;
                Ok((checked_left, Type::Bool, checked_right, Type::Bool))
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let (checked_left, left_ty) = self.expression(left)?;
                let (checked_right, right_ty) = self.expression(right)?;
                let supported = matches!(
                    left_ty,
                    Type::Unit | Type::Bool | Type::Int | Type::Float | Type::String
                );
                if !supported {
                    return Err((
                        span,
                        TypeError::UnsupportedBinaryOp {
                            op,
                            left: left_ty,
                            right: right_ty,
                        },
                    ));
                }
                expect(&left_ty, right_ty, right.span)?;
                Ok((checked_left, left_ty, checked_right, Type::Bool))
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let (checked_left, left_ty) = self.expression(left)?;
                let (checked_right, right_ty) = self.expression(right)?;
                let supported = matches!(left_ty, Type::Int | Type::Float | Type::String);
                if !supported {
                    return Err((
                        span,
                        TypeError::UnsupportedBinaryOp {
                            op,
                            left: left_ty,
                            right: right_ty,
                        },
                    ));
                }
                expect(&left_ty, right_ty, right.span)?;
                Ok((checked_left, left_ty, checked_right, Type::Bool))
            }
            BinaryOp::Add => {
                let (checked_left, left_ty) = self.expression(left)?;
                let (checked_right, right_ty) = self.expression(right)?;
                let supported = matches!(left_ty, Type::Int | Type::Float | Type::String);
                if !supported {
                    return Err((
                        span,
                        TypeError::UnsupportedBinaryOp {
                            op,
                            left: left_ty,
                            right: right_ty,
                        },
                    ));
                }
                expect(&left_ty, right_ty, right.span)?;
                let result_ty = left_ty.clone();
                Ok((checked_left, left_ty, checked_right, result_ty))
            }
            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                let (checked_left, left_ty) = self.expression(left)?;
                let (checked_right, right_ty) = self.expression(right)?;
                let supported = matches!(left_ty, Type::Int | Type::Float);
                if !supported {
                    return Err((
                        span,
                        TypeError::UnsupportedBinaryOp {
                            op,
                            left: left_ty,
                            right: right_ty,
                        },
                    ));
                }
                expect(&left_ty, right_ty, right.span)?;
                let result_ty = left_ty.clone();
                Ok((checked_left, left_ty, checked_right, result_ty))
            }
        }
    }

    fn expect_expr(
        &mut self,
        expr: &'a Expr,
        expected: &Type,
    ) -> Result<ModuleExpr, (Span, TypeError)> {
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
