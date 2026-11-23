use crate::ast::{
    AttrEntry, AttrLiteral, BinaryOp, BlockPiece, Decl, Expr, ExprKind, Function, InlinePiece,
    Method, Module, Program, Span, Type, TypePattern, UnaryOp, WhereExpr, WhereExprKind,
};
use crate::builtin::{BuiltinFn, lookup_builtin_method, lookup_prelude_function};
use crate::content::{AttrValue, Attrs};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ModuleId(pub(crate) usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DeclId {
    pub(crate) module: ModuleId,
    pub(crate) index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct InstanceKey {
    pub(crate) decl: DeclId,
    pub(crate) type_args: Vec<Type>,
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
    Builtin(BuiltinFn, Span),
    InlineFragment(Vec<Self>, Attrs, Span),
    BlockFragment(Vec<Self>, Attrs, Span),
    BlockList(bool, Vec<Self>, Attrs, Span),
    BlockTable {
        headers: Vec<Self>,
        rows: Vec<Vec<Self>>,
        attrs: Attrs,
        span: Span,
    },
    List(Vec<Self>),
    For(Box<Self>, Box<Self>),
    CustomNode {
        is_block: bool,
        module: Arc<str>,
        name: Arc<str>,
        props: Vec<(Arc<str>, Self)>,
        span: Span,
    },
    AttachAttrs(Box<Self>, Attrs),
    Let(Box<Self>, Box<Self>),
    Seq(Box<Self>, Box<Self>),
    Unary(UnaryOp, Box<Self>),
    Binary(BinaryOp, Box<Self>, Box<Self>),
    If(Box<Self>, Box<Self>, Box<Self>),
    Call(Box<Self>, Vec<Self>),
}

/// Checked expression before specialization and entry-reachability lowering.
/// Uses module-qualified `InstanceKey` so per-module checking never depends on
/// a global flat function layout.
#[derive(Debug)]
pub(crate) enum ModuleExpr {
    Unit,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(Arc<str>),
    Local(usize),
    Function(InstanceKey, Span),
    Builtin(BuiltinFn, Span),
    InlineFragment(Vec<Self>, Attrs, Span),
    BlockFragment(Vec<Self>, Attrs, Span),
    BlockList(bool, Vec<Self>, Attrs, Span),
    BlockTable {
        headers: Vec<Self>,
        rows: Vec<Vec<Self>>,
        attrs: Attrs,
        span: Span,
    },
    List(Vec<Self>),
    For(Box<Self>, Box<Self>),
    CustomNode {
        return_ty: Type,
        module: Arc<str>,
        name: Arc<str>,
        props: Vec<(Arc<str>, Self)>,
        span: Span,
    },
    AttachAttrs(Box<Self>, Attrs),
    Let(Box<Self>, Box<Self>),
    Seq(Box<Self>, Box<Self>),
    Unary(UnaryOp, Box<Self>),
    Binary(BinaryOp, Box<Self>, Box<Self>),
    If(Box<Self>, Box<Self>, Box<Self>),
    TypeMatch {
        param_index: usize,
        arms: Vec<(Type, Self)>,
        wildcard: Box<Self>,
    },
    CompileError {
        message: String,
        span: Span,
    },
    Call(Box<Self>, Vec<Self>),
}

#[derive(Debug)]
pub(crate) struct CheckedDecl {
    pub(crate) name: String,
    pub(crate) type_params: Vec<String>,
    pub(crate) where_clause: Option<WhereExpr>,
    pub(crate) arity: usize,
    pub(crate) body: ModuleExpr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Binding {
    Function(DeclId),
    Namespace(ModuleId),
}

#[derive(Debug, Clone)]
pub(crate) struct FunctionSig {
    pub(crate) name: String,
    pub(crate) type_params: Vec<String>,
    pub(crate) parameters: Vec<Type>,
    pub(crate) returns: Type,
    pub(crate) where_clause: Option<WhereExpr>,
}

#[derive(Debug, Clone)]
pub(crate) struct MethodSig {
    pub(crate) id: DeclId,
    pub(crate) parameters: Vec<Type>,
    pub(crate) returns: Type,
}

/// Body-independent lexical scope for checking a single module.
#[derive(Debug, Clone)]
pub(crate) struct ModuleScope {
    pub(crate) bindings: HashMap<String, Binding>,
    pub(crate) methods: HashMap<(Type, String), MethodSig>,
}

/// Project-wide signature & export view read by per-module body checking.
/// Contains no function bodies, keeping body checking isolated per module.
pub(crate) struct InterfaceView<'a> {
    pub(crate) module_paths: &'a [String],
    pub(crate) exports: &'a [HashMap<String, Binding>],
    pub(crate) signatures: &'a [Vec<FunctionSig>],
}

impl InterfaceView<'_> {
    fn signature(&self, id: DeclId) -> &FunctionSig {
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
    DuplicateTypeParam(String),
    UnknownType(String),
    UnknownTypeParam(String),
    TypeArity { expected: usize, actual: usize },
    TypeArgsRequired { function: String, expected: usize },
    CannotInferTypeParam { function: String, param: String },
    WhereUnsatisfied { function: String },
    DuplicateMatchPattern(Type),
    DuplicateWildcardArm,
    MissingWildcardArm,
    CompileError(String),
    InvalidExternReturn(Type),
    InvalidExternParam(Type),
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
    NotIterable(Type),
    CannotInferEmptyList,
    Arity { expected: usize, actual: usize },
    Mismatch { expected: Type, actual: Type },
}

/// Body-independent signature summary extracted from a single parsed module.
pub(crate) struct ModuleInterface {
    pub(crate) signatures: Vec<FunctionSig>,
    pub(crate) bindings: HashMap<String, Binding>,
    pub(crate) methods: HashMap<(Type, String), MethodSig>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

/// Collect all signatures first, then check every body (including unused and
/// generic definitions), independent of reachability.
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

    let (checked_decls, body_diags) = check_module_bodies("<root>", &program.root, &scope, &view);
    diagnostics.extend(body_diags);
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    let all_decls = vec![checked_decls];
    let mut roots: Vec<(String, InstanceKey)> = Vec::new();
    for (name, binding) in &scope.bindings {
        if let Binding::Function(decl_id) = *binding
            && all_decls[0][decl_id.index].type_params.is_empty()
        {
            roots.push((
                name.clone(),
                InstanceKey {
                    decl: decl_id,
                    type_args: Vec::new(),
                },
            ));
        }
    }
    roots.sort_by(|(a, _), (b, _)| a.cmp(b));

    specialize_roots(&all_decls, &roots).map_err(|errs| {
        errs.into_iter()
            .map(|(_, diag)| diag)
            .collect::<Vec<_>>()
    })
}

/// Specialize a set of concrete root instances and all transitively reachable
/// instances into a dense `CheckedProgram`, collapsing `TypeMatch` statically
/// and rejecting any reachable `compile_error(...)`.
pub(crate) fn specialize_roots(
    decls: &[Vec<CheckedDecl>],
    roots: &[(String, InstanceKey)],
) -> Result<CheckedProgram, Vec<(ModuleId, Diagnostic)>> {
    let mut mapping: HashMap<InstanceKey, FunctionId> = HashMap::new();
    let mut order: Vec<InstanceKey> = Vec::new();
    let mut queue: VecDeque<InstanceKey> = VecDeque::new();
    let mut function_names = HashMap::with_capacity(roots.len());

    for (entry_name, key) in roots {
        let id = match mapping.entry(key.clone()) {
            std::collections::hash_map::Entry::Occupied(e) => *e.get(),
            std::collections::hash_map::Entry::Vacant(e) => {
                let next = FunctionId(order.len());
                e.insert(next);
                order.push(key.clone());
                queue.push_back(key.clone());
                next
            }
        };
        function_names.insert(entry_name.clone(), id);
    }

    let mut lowered_bodies: Vec<Option<CheckedFunction>> = Vec::new();
    let mut errors: Vec<(ModuleId, Diagnostic)> = Vec::new();

    while let Some(instance) = queue.pop_front() {
        let fn_id = mapping[&instance];
        let decl = &decls[instance.decl.module.0][instance.decl.index];
        let body = lower_specialized_expr(
            decls,
            &decl.body,
            &decl.type_params,
            &instance.type_args,
            &mut mapping,
            &mut order,
            &mut queue,
        );
        match body {
            Ok(body) => {
                if lowered_bodies.len() <= fn_id.0 {
                    lowered_bodies.resize_with(fn_id.0 + 1, || None);
                }
                lowered_bodies[fn_id.0] = Some(CheckedFunction {
                    arity: decl.arity,
                    body,
                });
            }
            Err((span, error)) => {
                errors.push((
                    instance.decl.module,
                    Diagnostic {
                        declaration: decl.name.clone(),
                        span,
                        error,
                    },
                ));
            }
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    let functions = lowered_bodies
        .into_iter()
        .map(|slot| slot.expect("every queued instance was lowered"))
        .collect();

    Ok(CheckedProgram {
        function_names,
        functions,
    })
}

fn lower_specialized_expr(
    decls: &[Vec<CheckedDecl>],
    expr: &ModuleExpr,
    type_params: &[String],
    type_args: &[Type],
    mapping: &mut HashMap<InstanceKey, FunctionId>,
    order: &mut Vec<InstanceKey>,
    queue: &mut VecDeque<InstanceKey>,
) -> Result<CheckedExpr, (Span, TypeError)> {
    use CheckedExpr as C;
    use ModuleExpr as M;
    Ok(match expr {
        M::Unit => C::Unit,
        M::Bool(b) => C::Bool(*b),
        M::Int(n) => C::Int(*n),
        M::Float(f) => C::Float(*f),
        M::String(s) => C::String(Arc::clone(s)),
        M::Local(slot) => C::Local(*slot),
        M::Function(key, span) => {
            let concrete_args: Vec<Type> = key
                .type_args
                .iter()
                .map(|ty| substitute_type(ty, type_params, type_args))
                .collect();
            let target_decl = &decls[key.decl.module.0][key.decl.index];
            if let Some(where_expr) = &target_decl.where_clause
                && eval_where(where_expr, &target_decl.type_params, &concrete_args) != Some(true)
            {
                return Err((
                    *span,
                    TypeError::WhereUnsatisfied {
                        function: target_decl.name.clone(),
                    },
                ));
            }
            let concrete_key = InstanceKey {
                decl: key.decl,
                type_args: concrete_args,
            };
            let id = match mapping.entry(concrete_key.clone()) {
                std::collections::hash_map::Entry::Occupied(e) => *e.get(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let next = FunctionId(order.len());
                    e.insert(next);
                    order.push(concrete_key.clone());
                    queue.push_back(concrete_key);
                    next
                }
            };
            C::Function(id)
        }
        M::Builtin(builtin, span) => C::Builtin(*builtin, *span),
        M::InlineFragment(items, attrs, span) => {
            let mut lowered = Vec::with_capacity(items.len());
            for item in items {
                lowered.push(lower_specialized_expr(
                    decls,
                    item,
                    type_params,
                    type_args,
                    mapping,
                    order,
                    queue,
                )?);
            }
            C::InlineFragment(lowered, attrs.clone(), *span)
        }
        M::BlockFragment(items, attrs, span) => {
            let mut lowered = Vec::with_capacity(items.len());
            for item in items {
                lowered.push(lower_specialized_expr(
                    decls,
                    item,
                    type_params,
                    type_args,
                    mapping,
                    order,
                    queue,
                )?);
            }
            C::BlockFragment(lowered, attrs.clone(), *span)
        }
        M::BlockList(ordered, items, attrs, span) => {
            let mut lowered = Vec::with_capacity(items.len());
            for item in items {
                lowered.push(lower_specialized_expr(
                    decls,
                    item,
                    type_params,
                    type_args,
                    mapping,
                    order,
                    queue,
                )?);
            }
            C::BlockList(*ordered, lowered, attrs.clone(), *span)
        }
        M::BlockTable {
            headers,
            rows,
            attrs,
            span,
        } => {
            let mut lowered_headers = Vec::with_capacity(headers.len());
            for cell in headers {
                lowered_headers.push(lower_specialized_expr(
                    decls,
                    cell,
                    type_params,
                    type_args,
                    mapping,
                    order,
                    queue,
                )?);
            }
            let mut lowered_rows = Vec::with_capacity(rows.len());
            for row in rows {
                let mut lowered_row = Vec::with_capacity(row.len());
                for cell in row {
                    lowered_row.push(lower_specialized_expr(
                        decls,
                        cell,
                        type_params,
                        type_args,
                        mapping,
                        order,
                        queue,
                    )?);
                }
                lowered_rows.push(lowered_row);
            }
            C::BlockTable {
                headers: lowered_headers,
                rows: lowered_rows,
                attrs: attrs.clone(),
                span: *span,
            }
        }
        M::List(items) => {
            let mut lowered = Vec::with_capacity(items.len());
            for item in items {
                lowered.push(lower_specialized_expr(
                    decls,
                    item,
                    type_params,
                    type_args,
                    mapping,
                    order,
                    queue,
                )?);
            }
            C::List(lowered)
        }
        M::For(iterable, body) => C::For(
            Box::new(lower_specialized_expr(
                decls,
                iterable,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
            Box::new(lower_specialized_expr(
                decls,
                body,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
        ),
        M::CustomNode {
            return_ty,
            module,
            name,
            props,
            span,
        } => {
            let concrete_ret = substitute_type(return_ty, type_params, type_args);
            let is_block = match concrete_ret {
                Type::Inline => false,
                Type::Block => true,
                other => return Err((*span, TypeError::InvalidExternReturn(other))),
            };
            let mut lowered_props = Vec::with_capacity(props.len());
            for (k, v) in props {
                let lv = lower_specialized_expr(
                    decls,
                    v,
                    type_params,
                    type_args,
                    mapping,
                    order,
                    queue,
                )?;
                lowered_props.push((Arc::clone(k), lv));
            }
            C::CustomNode {
                is_block,
                module: Arc::clone(module),
                name: Arc::clone(name),
                props: lowered_props,
                span: *span,
            }
        }
        M::AttachAttrs(target, attrs) => C::AttachAttrs(
            Box::new(lower_specialized_expr(
                decls,
                target,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
            attrs.clone(),
        ),
        M::Let(init, body) => C::Let(
            Box::new(lower_specialized_expr(
                decls,
                init,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
            Box::new(lower_specialized_expr(
                decls,
                body,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
        ),
        M::Seq(first, second) => C::Seq(
            Box::new(lower_specialized_expr(
                decls,
                first,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
            Box::new(lower_specialized_expr(
                decls,
                second,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
        ),
        M::Unary(op, operand) => C::Unary(
            *op,
            Box::new(lower_specialized_expr(
                decls,
                operand,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
        ),
        M::Binary(op, left, right) => C::Binary(
            *op,
            Box::new(lower_specialized_expr(
                decls,
                left,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
            Box::new(lower_specialized_expr(
                decls,
                right,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
        ),
        M::If(cond, yes, no) => C::If(
            Box::new(lower_specialized_expr(
                decls,
                cond,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
            Box::new(lower_specialized_expr(
                decls,
                yes,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
            Box::new(lower_specialized_expr(
                decls,
                no,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?),
        ),
        M::TypeMatch {
            param_index,
            arms,
            wildcard,
        } => {
            let concrete_ty = &type_args[*param_index];
            let selected = arms
                .iter()
                .find(|(pat_ty, _)| substitute_type(pat_ty, type_params, type_args) == *concrete_ty)
                .map(|(_, body)| body)
                .unwrap_or(wildcard.as_ref());
            lower_specialized_expr(
                decls,
                selected,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?
        }
        M::CompileError { message, span } => {
            return Err((*span, TypeError::CompileError(message.clone())));
        }
        M::Call(callee, args) => {
            let callee = lower_specialized_expr(
                decls,
                callee,
                type_params,
                type_args,
                mapping,
                order,
                queue,
            )?;
            let mut lowered_args = Vec::with_capacity(args.len());
            for arg in args {
                lowered_args.push(lower_specialized_expr(
                    decls,
                    arg,
                    type_params,
                    type_args,
                    mapping,
                    order,
                    queue,
                )?);
            }
            C::Call(Box::new(callee), lowered_args)
        }
    })
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

                let mut seen_tparams = HashSet::new();
                let mut type_params = Vec::with_capacity(function.type_params.len());
                for tp in &function.type_params {
                    if !seen_tparams.insert(tp.name.as_str()) {
                        diagnostics.push(Diagnostic {
                            declaration: function.name.clone(),
                            span: tp.span,
                            error: TypeError::DuplicateTypeParam(tp.name.clone()),
                        });
                    }
                    type_params.push(tp.name.clone());
                }

                for param in &function.parameters {
                    if let Err(unknown) = validate_type(&param.ty, &type_params) {
                        diagnostics.push(Diagnostic {
                            declaration: function.name.clone(),
                            span: param.span,
                            error: TypeError::UnknownType(unknown),
                        });
                    }
                }
                if let Err(unknown) = validate_type(&function.returns, &type_params) {
                    diagnostics.push(Diagnostic {
                        declaration: function.name.clone(),
                        span: function.name_span,
                        error: TypeError::UnknownType(unknown),
                    });
                }
                if let Some(where_expr) = &function.where_clause
                    && let Err(unknown) = validate_where(where_expr, &type_params)
                {
                    diagnostics.push(Diagnostic {
                        declaration: function.name.clone(),
                        span: where_expr.span,
                        error: TypeError::UnknownType(unknown),
                    });
                }

                signatures.push(FunctionSig {
                    name: function.name.clone(),
                    type_params,
                    parameters: function.parameters.iter().map(|p| p.ty.clone()).collect(),
                    returns: function.returns.clone(),
                    where_clause: function.where_clause.clone(),
                });
            }
            Decl::Method(method) => {
                let id = DeclId {
                    module: module_id,
                    index: signatures.len(),
                };
                let decl_name = method_declaration_name(method);
                if let Err(unknown) = validate_type(&method.receiver_type, &[]) {
                    diagnostics.push(Diagnostic {
                        declaration: decl_name.clone(),
                        span: method.name_span,
                        error: TypeError::UnknownType(unknown),
                    });
                }
                for param in &method.parameters {
                    if let Err(unknown) = validate_type(&param.ty, &[]) {
                        diagnostics.push(Diagnostic {
                            declaration: decl_name.clone(),
                            span: param.span,
                            error: TypeError::UnknownType(unknown),
                        });
                    }
                }
                if let Err(unknown) = validate_type(&method.returns, &[]) {
                    diagnostics.push(Diagnostic {
                        declaration: decl_name.clone(),
                        span: method.name_span,
                        error: TypeError::UnknownType(unknown),
                    });
                }

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
                        declaration: decl_name.clone(),
                        span: method.name_span,
                        error: TypeError::DuplicateMethod,
                    });
                }
                let mut full_params = Vec::with_capacity(1 + parameters.len());
                full_params.push(method.receiver_type.clone());
                full_params.extend(parameters);
                signatures.push(FunctionSig {
                    name: decl_name,
                    type_params: Vec::new(),
                    parameters: full_params,
                    returns: method.returns.clone(),
                    where_clause: None,
                });
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

fn validate_type(ty: &Type, type_params: &[String]) -> Result<(), String> {
    match ty {
        Type::Unit
        | Type::Bool
        | Type::Int
        | Type::Float
        | Type::String
        | Type::Inline
        | Type::Block
        | Type::Json
        | Type::Never => Ok(()),
        Type::List(elem) => validate_type(elem, type_params),
        Type::Param(name) => {
            if type_params.iter().any(|p| p == name) {
                Ok(())
            } else {
                Err(name.clone())
            }
        }
        Type::Function(params, ret) => {
            for p in params {
                validate_type(p, type_params)?;
            }
            validate_type(ret, type_params)
        }
    }
}

fn validate_where(expr: &WhereExpr, type_params: &[String]) -> Result<(), String> {
    match &expr.kind {
        WhereExprKind::Bool(_) => Ok(()),
        WhereExprKind::TypeEq(a, b) | WhereExprKind::TypeNe(a, b) => {
            validate_type(a, type_params)?;
            validate_type(b, type_params)
        }
        WhereExprKind::Not(inner) => validate_where(inner, type_params),
        WhereExprKind::And(a, b) | WhereExprKind::Or(a, b) => {
            validate_where(a, type_params)?;
            validate_where(b, type_params)
        }
    }
}

/// Evaluate a `where` guard against substituted type arguments.
/// Returns `None` if an abstract (un-narrowed) type parameter prevents proving
/// the guard statically.
fn eval_where(expr: &WhereExpr, params: &[String], args: &[Type]) -> Option<bool> {
    match &expr.kind {
        WhereExprKind::Bool(b) => Some(*b),
        WhereExprKind::TypeEq(a, b) => {
            let a = substitute_type(a, params, args);
            let b = substitute_type(b, params, args);
            if a == b {
                Some(true)
            } else if contains_param(&a) || contains_param(&b) {
                None
            } else {
                Some(false)
            }
        }
        WhereExprKind::TypeNe(a, b) => {
            let a = substitute_type(a, params, args);
            let b = substitute_type(b, params, args);
            if a == b {
                Some(false)
            } else if contains_param(&a) || contains_param(&b) {
                None
            } else {
                Some(true)
            }
        }
        WhereExprKind::Not(inner) => eval_where(inner, params, args).map(|b| !b),
        WhereExprKind::And(a, b) => match (eval_where(a, params, args), eval_where(b, params, args))
        {
            (Some(false), _) | (_, Some(false)) => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        },
        WhereExprKind::Or(a, b) => match (eval_where(a, params, args), eval_where(b, params, args))
        {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        },
    }
}

pub(crate) fn substitute_type(ty: &Type, params: &[String], args: &[Type]) -> Type {
    match ty {
        Type::Unit
        | Type::Bool
        | Type::Int
        | Type::Float
        | Type::String
        | Type::Inline
        | Type::Block
        | Type::Json
        | Type::Never => ty.clone(),
        Type::List(elem) => Type::List(Box::new(substitute_type(elem, params, args))),
        Type::Param(name) => params
            .iter()
            .position(|p| p == name)
            .map_or_else(|| ty.clone(), |idx| args[idx].clone()),
        Type::Function(ps, ret) => Type::Function(
            ps.iter().map(|p| substitute_type(p, params, args)).collect(),
            Box::new(substitute_type(ret, params, args)),
        ),
    }
}

fn substitute_map(ty: &Type, subst: &HashMap<String, Type>) -> Type {
    if subst.is_empty() {
        return ty.clone();
    }
    match ty {
        Type::Unit
        | Type::Bool
        | Type::Int
        | Type::Float
        | Type::String
        | Type::Inline
        | Type::Block
        | Type::Json
        | Type::Never => ty.clone(),
        Type::List(elem) => Type::List(Box::new(substitute_map(elem, subst))),
        Type::Param(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Function(ps, ret) => Type::Function(
            ps.iter().map(|p| substitute_map(p, subst)).collect(),
            Box::new(substitute_map(ret, subst)),
        ),
    }
}

fn contains_param(ty: &Type) -> bool {
    match ty {
        Type::Param(_) => true,
        Type::List(elem) => contains_param(elem),
        Type::Function(ps, ret) => ps.iter().any(contains_param) || contains_param(ret),
        _ => false,
    }
}

fn unify_type(
    pattern: &Type,
    actual: &Type,
    params: &[String],
    inferred: &mut [Option<Type>],
) {
    if *actual == Type::Never {
        return;
    }
    match pattern {
        Type::Param(name) => {
            if let Some(idx) = params.iter().position(|p| p == name)
                && inferred[idx].is_none()
            {
                inferred[idx] = Some(actual.clone());
            }
        }
        Type::List(pat_elem) => {
            if let Type::List(act_elem) = actual {
                unify_type(pat_elem, act_elem, params, inferred);
            }
        }
        Type::Function(pat_ps, pat_ret) => {
            if let Type::Function(act_ps, act_ret) = actual
                && pat_ps.len() == act_ps.len()
            {
                for (p, a) in pat_ps.iter().zip(act_ps) {
                    unify_type(p, a, params, inferred);
                }
                unify_type(pat_ret, act_ret, params, inferred);
            }
        }
        _ => {}
    }
}

/// Layer 2: check every function and method body in a single module using only
/// its resolved `ModuleScope` and project `InterfaceView` (never reading other
/// modules' bodies).
pub(crate) fn check_module_bodies(
    module_path: &str,
    module: &Module,
    scope: &ModuleScope,
    view: &InterfaceView<'_>,
) -> (Vec<CheckedDecl>, Vec<Diagnostic>) {
    let mut checked = Vec::new();
    let mut diagnostics = Vec::new();

    for decl in &module.declarations {
        let (declaration, result) = match decl {
            Decl::Import(_) => continue,
            Decl::Function(function) => {
                let tparams: Vec<String> = function
                    .type_params
                    .iter()
                    .map(|tp| tp.name.clone())
                    .collect();
                let mut checker = Checker {
                    module_path,
                    scope,
                    view,
                    type_params: &tparams,
                    type_subst: HashMap::new(),
                    locals: HashMap::new(),
                    depth: 0,
                };
                (function.name.clone(), checker.function(function))
            }
            Decl::Method(method) => {
                let mut checker = Checker {
                    module_path,
                    scope,
                    view,
                    type_params: &[],
                    type_subst: HashMap::new(),
                    locals: HashMap::new(),
                    depth: 0,
                };
                (method_declaration_name(method), checker.method(method))
            }
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

pub fn format_type(ty: &Type) -> String {
    match ty {
        Type::Unit => "Unit".into(),
        Type::Bool => "Bool".into(),
        Type::Int => "Int".into(),
        Type::Float => "Float".into(),
        Type::String => "String".into(),
        Type::Inline => "Inline".into(),
        Type::Block => "Block".into(),
        Type::Json => "Json".into(),
        Type::List(elem) => format!("List<{}>", format_type(elem)),
        Type::Never => "Never".into(),
        Type::Param(name) => name.clone(),
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

pub fn format_type_error(error: &TypeError) -> String {
    match error {
        TypeError::DuplicateFunction => "duplicate function declaration".into(),
        TypeError::DuplicateMethod => "duplicate method declaration for receiver type".into(),
        TypeError::DuplicateBinding(name) => format!("duplicate binding `{name}`"),
        TypeError::DuplicateParameter(name) => format!("duplicate parameter `{name}`"),
        TypeError::DuplicateTypeParam(name) => format!("duplicate type parameter `{name}`"),
        TypeError::UnknownType(name) => format!("unknown type `{name}`"),
        TypeError::UnknownTypeParam(name) => format!("unknown type parameter `{name}`"),
        TypeError::TypeArity { expected, actual } => {
            format!("expected {expected} type argument(s), found {actual}")
        }
        TypeError::TypeArgsRequired { function, expected } => format!(
            "generic function `{function}` requires {expected} type argument(s) when used as a value"
        ),
        TypeError::CannotInferTypeParam { function, param } => {
            format!("cannot infer type parameter `{param}` for `{function}`")
        }
        TypeError::WhereUnsatisfied { function } => {
            format!("`where` constraint on `{function}` is not satisfied")
        }
        TypeError::DuplicateMatchPattern(ty) => {
            format!("duplicate `match` arm for type `{}`", format_type(ty))
        }
        TypeError::DuplicateWildcardArm => "duplicate wildcard `_` arm in `match`".into(),
        TypeError::MissingWildcardArm => {
            "`match` on type parameter requires a `_` fallback arm".into()
        }
        TypeError::CompileError(msg) => format!("compile_error: {msg}"),
        TypeError::InvalidExternReturn(ty) => format!(
            "bodyless external content declaration must return `Inline` or `Block`, found `{}`",
            format_type(ty)
        ),
        TypeError::InvalidExternParam(ty) => format!(
            "external content declaration parameter cannot have function type `{}`",
            format_type(ty)
        ),
        TypeError::InvalidImportPath(path) => format!("invalid relative import path `{path}`"),
        TypeError::InvalidImportAlias(alias) => {
            format!("cannot derive valid module alias from `{alias}`")
        }
        TypeError::MissingModule(path) => format!("missing imported module `{path}`"),
        TypeError::ImportCycle(cycle) => format!("import cycle: {}", cycle.join(" -> ")),
        TypeError::UnknownExport { module, name } => {
            format!("module `{module}` does not export `{name}`")
        }
        TypeError::AmbiguousMethod { receiver, method } => format!(
            "ambiguous imported method `{method}` on type `{}`",
            format_type(receiver)
        ),
        TypeError::NamespaceAsValue(name) => {
            format!("module namespace `{name}` cannot be used as a value")
        }
        TypeError::NotNamespace(ty) => format!(
            "cannot access member on non-namespace value of type `{}`",
            format_type(ty)
        ),
        TypeError::UnknownName(name) => format!("unknown name `{name}`"),
        TypeError::UnknownMethod { receiver, method } => {
            format!("no method `{method}` on type `{}`", format_type(receiver))
        }
        TypeError::UnsupportedUnaryOp { op, operand } => {
            format!(
                "operator `{op:?}` cannot be applied to `{}`",
                format_type(operand)
            )
        }
        TypeError::UnsupportedBinaryOp { op, left, right } => format!(
            "operator `{op:?}` cannot be applied to `{}` and `{}`",
            format_type(left),
            format_type(right)
        ),
        TypeError::NotCallable(ty) => format!("type `{}` is not callable", format_type(ty)),
        TypeError::NotIterable(ty) => format!(
            "expected `List<T>` in `for` expression, found `{}`",
            format_type(ty)
        ),
        TypeError::CannotInferEmptyList => {
            "cannot infer element type of empty list literal `#[ ]`".into()
        }
        TypeError::Arity { expected, actual } => {
            format!("expected {expected} argument(s), found {actual}")
        }
        TypeError::Mismatch { expected, actual } => format!(
            "type mismatch: expected `{}`, found `{}`",
            format_type(expected),
            format_type(actual)
        ),
    }
}

struct Checker<'a> {
    module_path: &'a str,
    scope: &'a ModuleScope,
    view: &'a InterfaceView<'a>,
    type_params: &'a [String],
    type_subst: HashMap<String, Type>,
    locals: HashMap<&'a str, (usize, Type)>,
    depth: usize,
}

impl<'a> Checker<'a> {
    fn resolve_ty(&self, ty: &Type) -> Type {
        substitute_map(ty, &self.type_subst)
    }

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
        let body = match &function.body {
            Some(expr) => self.expect_expr(expr, &function.returns)?,
            None => {
                if !matches!(function.returns, Type::Inline | Type::Block | Type::Param(_)) {
                    return Err((
                        function.name_span,
                        TypeError::InvalidExternReturn(function.returns.clone()),
                    ));
                }
                let mut props = Vec::with_capacity(function.parameters.len());
                for (slot, param) in function.parameters.iter().enumerate() {
                    if matches!(param.ty, Type::Function(_, _)) {
                        return Err((
                            param.span,
                            TypeError::InvalidExternParam(param.ty.clone()),
                        ));
                    }
                    props.push((Arc::from(param.name.as_str()), ModuleExpr::Local(slot)));
                }
                ModuleExpr::CustomNode {
                    return_ty: function.returns.clone(),
                    module: Arc::from(self.module_path),
                    name: Arc::from(function.name.as_str()),
                    props,
                    span: function.name_span,
                }
            }
        };
        Ok(CheckedDecl {
            name: function.name.clone(),
            type_params: self.type_params.to_vec(),
            where_clause: function.where_clause.clone(),
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
        let body = self.expect_expr(&method.body, &method.returns)?;
        Ok(CheckedDecl {
            name: method_declaration_name(method),
            type_params: Vec::new(),
            where_clause: None,
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

    /// Resolve a direct function target (`Name` or namespace `Member`), returning
    /// `Ok(None)` if shadowed by a local or not a static function target.
    fn resolve_function_target(&self, expr: &Expr) -> Result<Option<DeclId>, (Span, TypeError)> {
        match &expr.kind {
            ExprKind::Name(name) => {
                if self.locals.contains_key(name.as_str()) {
                    return Ok(None);
                }
                match self.scope.bindings.get(name) {
                    Some(Binding::Function(id)) => Ok(Some(*id)),
                    Some(Binding::Namespace(_)) => {
                        Err((expr.span, TypeError::NamespaceAsValue(name.clone())))
                    }
                    None => Ok(None),
                }
            }
            ExprKind::Member(base, member) => {
                let Some(module_id) = self.resolve_namespace(base)? else {
                    return Ok(None);
                };
                match self.view.exports[module_id.0].get(member) {
                    Some(Binding::Function(id)) => Ok(Some(*id)),
                    Some(Binding::Namespace(_)) => {
                        Err((expr.span, TypeError::NamespaceAsValue(member.clone())))
                    }
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

    fn instantiate_function(
        &self,
        decl_id: DeclId,
        raw_type_args: &[Type],
        span: Span,
    ) -> Result<(ModuleExpr, Type), (Span, TypeError)> {
        let sig = self.view.signature(decl_id);
        if sig.type_params.len() != raw_type_args.len() {
            return Err((
                span,
                TypeError::TypeArity {
                    expected: sig.type_params.len(),
                    actual: raw_type_args.len(),
                },
            ));
        }
        let mut type_args = Vec::with_capacity(raw_type_args.len());
        for ty in raw_type_args {
            if let Err(unknown) = validate_type(ty, self.type_params) {
                return Err((span, TypeError::UnknownType(unknown)));
            }
            type_args.push(self.resolve_ty(ty));
        }
        if let Some(where_expr) = &sig.where_clause
            && eval_where(where_expr, &sig.type_params, &type_args) == Some(false)
        {
            return Err((
                span,
                TypeError::WhereUnsatisfied {
                    function: sig.name.clone(),
                },
            ));
        }
        let concrete_params = sig
            .parameters
            .iter()
            .map(|p| substitute_type(p, &sig.type_params, &type_args))
            .collect();
        let concrete_ret = substitute_type(&sig.returns, &sig.type_params, &type_args);
        Ok((
            ModuleExpr::Function(
                InstanceKey {
                    decl: decl_id,
                    type_args,
                },
                span,
            ),
            Type::Function(concrete_params, Box::new(concrete_ret)),
        ))
    }

    fn call_generic_function(
        &mut self,
        decl_id: DeclId,
        arguments: &'a [Expr],
        call_span: Span,
    ) -> Result<(ModuleExpr, Type), (Span, TypeError)> {
        let sig = self.view.signature(decl_id).clone();
        if sig.parameters.len() != arguments.len() {
            return Err((
                call_span,
                TypeError::Arity {
                    expected: sig.parameters.len(),
                    actual: arguments.len(),
                },
            ));
        }

        let mut checked_args = Vec::with_capacity(arguments.len());
        let mut arg_types = Vec::with_capacity(arguments.len());
        for arg in arguments {
            let (checked, ty) = self.expression(arg, None)?;
            checked_args.push(checked);
            arg_types.push(ty);
        }

        let mut inferred: Vec<Option<Type>> = vec![None; sig.type_params.len()];
        for (pat, act) in sig.parameters.iter().zip(&arg_types) {
            unify_type(pat, act, &sig.type_params, &mut inferred);
        }

        let mut type_args = Vec::with_capacity(sig.type_params.len());
        for (idx, maybe) in inferred.into_iter().enumerate() {
            match maybe {
                Some(ty) => type_args.push(ty),
                None => {
                    return Err((
                        call_span,
                        TypeError::CannotInferTypeParam {
                            function: sig.name,
                            param: sig.type_params[idx].clone(),
                        },
                    ));
                }
            }
        }

        for ((arg, actual_ty), param_pat) in arguments
            .iter()
            .zip(arg_types)
            .zip(&sig.parameters)
        {
            let expected_ty = substitute_type(param_pat, &sig.type_params, &type_args);
            expect(&expected_ty, actual_ty, arg.span)?;
        }

        if let Some(where_expr) = &sig.where_clause
            && eval_where(where_expr, &sig.type_params, &type_args) == Some(false)
        {
            return Err((
                call_span,
                TypeError::WhereUnsatisfied {
                    function: sig.name,
                },
            ));
        }

        let returns = substitute_type(&sig.returns, &sig.type_params, &type_args);
        Ok((
            ModuleExpr::Call(
                Box::new(ModuleExpr::Function(
                    InstanceKey {
                        decl: decl_id,
                        type_args,
                    },
                    call_span,
                )),
                checked_args,
            ),
            returns,
        ))
    }

    fn expression(
        &mut self,
        expr: &'a Expr,
        expected: Option<&Type>,
    ) -> Result<(ModuleExpr, Type), (Span, TypeError)> {
        use ModuleExpr as M;
        Ok(match &expr.kind {
            ExprKind::Unit => (M::Unit, Type::Unit),
            ExprKind::Bool(b) => (M::Bool(*b), Type::Bool),
            ExprKind::Int(n) => (M::Int(*n), Type::Int),
            ExprKind::Float(f) => (M::Float(*f), Type::Float),
            ExprKind::String(s) => (M::String(Arc::from(s.as_str())), Type::String),
            ExprKind::InlineLiteral(pieces) => {
                (self.check_inline_pieces(pieces, expr.span)?, Type::Inline)
            }
            ExprKind::BlockLiteral(pieces) => {
                (self.check_block_pieces(pieces, expr.span)?, Type::Block)
            }
            ExprKind::ListLiteral(items) => {
                let expected_elem = expected.and_then(|ty| match self.resolve_ty(ty) {
                    Type::List(elem) => Some(*elem),
                    _ => None,
                });
                if let Some(elem_ty) = expected_elem {
                    let mut checked = Vec::with_capacity(items.len());
                    for item in items {
                        checked.push(self.expect_expr(item, &elem_ty)?);
                    }
                    (M::List(checked), Type::List(Box::new(elem_ty)))
                } else {
                    let Some((first, rest)) = items.split_first() else {
                        return Err((expr.span, TypeError::CannotInferEmptyList));
                    };
                    let (checked_first, elem_ty) = self.expression(first, None)?;
                    let mut checked = Vec::with_capacity(items.len());
                    checked.push(checked_first);
                    for item in rest {
                        checked.push(self.expect_expr(item, &elem_ty)?);
                    }
                    (M::List(checked), Type::List(Box::new(elem_ty)))
                }
            }
            ExprKind::For {
                binding,
                iterable,
                body,
                ..
            } => {
                let (checked_iter, iter_ty) = self.expression(iterable, None)?;
                let Type::List(elem_ty) = iter_ty else {
                    return Err((iterable.span, TypeError::NotIterable(iter_ty)));
                };
                let expected_body = expected.and_then(|ty| match self.resolve_ty(ty) {
                    Type::List(elem) => Some(*elem),
                    _ => None,
                });
                let previous = self.locals.insert(binding.as_str(), (self.depth, *elem_ty));
                self.depth += 1;
                let result = self.expression(body, expected_body.as_ref());
                self.depth -= 1;
                if let Some(previous) = previous {
                    self.locals.insert(binding.as_str(), previous);
                } else {
                    self.locals.remove(binding.as_str());
                }
                let (checked_body, body_ty) = result?;
                (
                    M::For(Box::new(checked_iter), Box::new(checked_body)),
                    Type::List(Box::new(body_ty)),
                )
            }
            ExprKind::AttachAttrs(attrs, target) => {
                let (checked, ty) = self.expression(target, expected)?;
                let lowered = lower_attrs(attrs);
                if lowered.is_empty() {
                    (checked, ty)
                } else {
                    (M::AttachAttrs(Box::new(checked), lowered), ty)
                }
            }
            ExprKind::CompileError(message) => (
                M::CompileError {
                    message: message.clone(),
                    span: expr.span,
                },
                Type::Never,
            ),
            ExprKind::Name(name) => {
                if let Some((slot, ty)) = self.locals.get(name.as_str()) {
                    (M::Local(*slot), self.resolve_ty(ty))
                } else if let Some(&binding) = self.scope.bindings.get(name) {
                    match binding {
                        Binding::Function(id) => {
                            let sig = self.view.signature(id);
                            if !sig.type_params.is_empty() {
                                return Err((
                                    expr.span,
                                    TypeError::TypeArgsRequired {
                                        function: sig.name.clone(),
                                        expected: sig.type_params.len(),
                                    },
                                ));
                            }
                            (
                                M::Function(
                                    InstanceKey {
                                        decl: id,
                                        type_args: Vec::new(),
                                    },
                                    expr.span,
                                ),
                                Type::Function(
                                    sig.parameters.clone(),
                                    Box::new(sig.returns.clone()),
                                ),
                            )
                        }
                        Binding::Namespace(_) => {
                            return Err((expr.span, TypeError::NamespaceAsValue(name.clone())));
                        }
                    }
                } else if let Some((builtin, params, returns)) = lookup_prelude_function(name) {
                    (
                        M::Builtin(builtin, expr.span),
                        Type::Function(params, Box::new(returns)),
                    )
                } else {
                    return Err((expr.span, TypeError::UnknownName(name.clone())));
                }
            }
            ExprKind::Member(base, member) => {
                if let Some(module_id) = self.resolve_namespace(base)? {
                    match self.view.exports[module_id.0].get(member) {
                        Some(&Binding::Function(id)) => {
                            let sig = self.view.signature(id);
                            if !sig.type_params.is_empty() {
                                return Err((
                                    expr.span,
                                    TypeError::TypeArgsRequired {
                                        function: sig.name.clone(),
                                        expected: sig.type_params.len(),
                                    },
                                ));
                            }
                            (
                                M::Function(
                                    InstanceKey {
                                        decl: id,
                                        type_args: Vec::new(),
                                    },
                                    expr.span,
                                ),
                                Type::Function(
                                    sig.parameters.clone(),
                                    Box::new(sig.returns.clone()),
                                ),
                            )
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
                    let (_, ty) = self.expression(base, None)?;
                    return Err((expr.span, TypeError::NotNamespace(ty)));
                }
            }
            ExprKind::Instantiate(target, raw_type_args) => {
                let Some(decl_id) = self.resolve_function_target(target)? else {
                    let (_, ty) = self.expression(target, None)?;
                    return Err((target.span, TypeError::NotCallable(ty)));
                };
                self.instantiate_function(decl_id, raw_type_args, expr.span)?
            }
            ExprKind::Let(name, initializer, body) => {
                let (initializer, ty) = self.expression(initializer, None)?;
                let previous = self.locals.insert(name, (self.depth, ty));
                self.depth += 1;
                let result = self.expression(body, expected);
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
                let (first, _) = self.expression(first, None)?;
                let (second, ty) = self.expression(second, expected)?;
                (M::Seq(Box::new(first), Box::new(second)), ty)
            }
            ExprKind::Unary(op, operand) => match op {
                UnaryOp::Not => {
                    let operand = self.expect_expr(operand, &Type::Bool)?;
                    (M::Unary(*op, Box::new(operand)), Type::Bool)
                }
                UnaryOp::Neg => {
                    let (checked, ty) = self.expression(operand, None)?;
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
                let (checked_left, checked_right, result_ty) =
                    self.check_binary(*op, left, right, expr.span)?;
                (
                    M::Binary(*op, Box::new(checked_left), Box::new(checked_right)),
                    result_ty,
                )
            }
            ExprKind::If(condition, yes, no) => {
                let condition = self.expect_expr(condition, &Type::Bool)?;
                let yes_span = yes.span;
                let (yes, yes_ty) = self.expression(yes, expected)?;
                let (no, ty) = if let Some(exp) = expected {
                    let resolved = self.resolve_ty(exp);
                    expect(&resolved, yes_ty, yes_span)?;
                    let no = self.expect_expr(no, &resolved)?;
                    (no, resolved)
                } else if yes_ty == Type::Never {
                    self.expression(no, None)?
                } else {
                    let no = self.expect_expr(no, &yes_ty)?;
                    (no, yes_ty)
                };
                (M::If(Box::new(condition), Box::new(yes), Box::new(no)), ty)
            }
            ExprKind::TypeMatch {
                param,
                param_span,
                arms,
            } => {
                let Some(param_index) = self.type_params.iter().position(|p| p == param) else {
                    return Err((*param_span, TypeError::UnknownTypeParam(param.clone())));
                };

                let mut seen_patterns: HashSet<Type> = HashSet::new();
                let mut checked_arms: Vec<(Type, ModuleExpr)> = Vec::new();
                let mut wildcard_body: Option<ModuleExpr> = None;
                let mut inferred_ty: Option<Type> = None;

                for arm in arms {
                    match &arm.pattern {
                        TypePattern::Type(raw_pat_ty) => {
                            if let Err(unknown) = validate_type(raw_pat_ty, self.type_params) {
                                return Err((arm.span, TypeError::UnknownType(unknown)));
                            }
                            let pat_ty = self.resolve_ty(raw_pat_ty);
                            if !seen_patterns.insert(pat_ty.clone()) {
                                return Err((arm.span, TypeError::DuplicateMatchPattern(pat_ty)));
                            }
                            let prev = self.type_subst.insert(param.clone(), pat_ty.clone());
                            let arm_result = if let Some(exp) = expected {
                                let resolved_exp = self.resolve_ty(exp);
                                self.expect_expr(&arm.body, &resolved_exp)
                                    .map(|e| (e, resolved_exp))
                            } else {
                                self.expression(&arm.body, None)
                            };
                            if let Some(prev) = prev {
                                self.type_subst.insert(param.clone(), prev);
                            } else {
                                self.type_subst.remove(param);
                            }
                            let (checked_body, arm_ty) = arm_result?;
                            if expected.is_none() && arm_ty != Type::Never {
                                if let Some(prev_ty) = &inferred_ty {
                                    expect(prev_ty, arm_ty, arm.body.span)?;
                                } else {
                                    inferred_ty = Some(arm_ty);
                                }
                            }
                            checked_arms.push((pat_ty, checked_body));
                        }
                        TypePattern::Wildcard => {
                            if wildcard_body.is_some() {
                                return Err((arm.span, TypeError::DuplicateWildcardArm));
                            }
                            let (checked_body, arm_ty) = if let Some(exp) = expected {
                                let resolved_exp = self.resolve_ty(exp);
                                let body = self.expect_expr(&arm.body, &resolved_exp)?;
                                (body, resolved_exp)
                            } else {
                                self.expression(&arm.body, None)?
                            };
                            if expected.is_none() && arm_ty != Type::Never {
                                if let Some(prev_ty) = &inferred_ty {
                                    expect(prev_ty, arm_ty, arm.body.span)?;
                                } else {
                                    inferred_ty = Some(arm_ty);
                                }
                            }
                            wildcard_body = Some(checked_body);
                        }
                    }
                }

                let Some(wildcard) = wildcard_body else {
                    return Err((expr.span, TypeError::MissingWildcardArm));
                };

                let result_ty = match expected {
                    Some(exp) => self.resolve_ty(exp),
                    None => inferred_ty.unwrap_or(Type::Never),
                };
                (
                    M::TypeMatch {
                        param_index,
                        arms: checked_arms,
                        wildcard: Box::new(wildcard),
                    },
                    result_ty,
                )
            }
            ExprKind::Call(callee, arguments) => {
                if let Some(decl_id) = self.resolve_function_target(callee)? {
                    let sig = self.view.signature(decl_id);
                    if !sig.type_params.is_empty() {
                        return self.call_generic_function(decl_id, arguments, expr.span);
                    }
                }
                let (checked_callee, ty) = self.expression(callee, None)?;
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
                            let sig = self.view.signature(id);
                            if !sig.type_params.is_empty() {
                                return self.call_generic_function(id, arguments, expr.span);
                            }
                            let parameters = sig.parameters.clone();
                            let returns = sig.returns.clone();
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
                            return Ok((
                                M::Call(
                                    Box::new(M::Function(
                                        InstanceKey {
                                            decl: id,
                                            type_args: Vec::new(),
                                        },
                                        expr.span,
                                    )),
                                    checked_args,
                                ),
                                returns,
                            ));
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

                let (receiver_expr, receiver_ty) = self.expression(receiver, None)?;
                if let Some(entry) = self
                    .scope
                    .methods
                    .get(&(receiver_ty.clone(), method_name.clone()))
                {
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
                        M::Call(
                            Box::new(M::Function(
                                InstanceKey {
                                    decl: entry.id,
                                    type_args: Vec::new(),
                                },
                                expr.span,
                            )),
                            call_args,
                        ),
                        entry.returns.clone(),
                    )
                } else if let Some((builtin, params, returns)) =
                    lookup_builtin_method(&receiver_ty, method_name)
                {
                    if params.len() != arguments.len() {
                        return Err((
                            expr.span,
                            TypeError::Arity {
                                expected: params.len(),
                                actual: arguments.len(),
                            },
                        ));
                    }
                    let mut call_args = Vec::with_capacity(1 + arguments.len());
                    call_args.push(receiver_expr);
                    for (arg, ty) in arguments.iter().zip(&params) {
                        call_args.push(self.expect_expr(arg, ty)?);
                    }
                    (
                        M::Call(Box::new(M::Builtin(builtin, expr.span)), call_args),
                        returns,
                    )
                } else {
                    return Err((
                        expr.span,
                        TypeError::UnknownMethod {
                            receiver: receiver_ty,
                            method: method_name.clone(),
                        },
                    ));
                }
            }
        })
    }

    fn check_inline_pieces(
        &mut self,
        pieces: &'a [InlinePiece],
        span: Span,
    ) -> Result<ModuleExpr, (Span, TypeError)> {
        let mut items = Vec::with_capacity(pieces.len());
        for piece in pieces {
            items.push(self.check_inline_piece(piece)?);
        }
        Ok(ModuleExpr::InlineFragment(items, Attrs::empty(), span))
    }

    fn check_inline_piece(
        &mut self,
        piece: &'a InlinePiece,
    ) -> Result<ModuleExpr, (Span, TypeError)> {
        use ModuleExpr as M;
        match piece {
            InlinePiece::Text(text, span) => Ok(M::Call(
                Box::new(M::Builtin(BuiltinFn::Text, *span)),
                vec![M::String(Arc::from(text.as_str()))],
            )),
            InlinePiece::Strong(items, span) => {
                let inner = self.check_inline_pieces(items, *span)?;
                Ok(M::Call(
                    Box::new(M::Builtin(BuiltinFn::Strong, *span)),
                    vec![inner],
                ))
            }
            InlinePiece::Emph(items, span) => {
                let inner = self.check_inline_pieces(items, *span)?;
                Ok(M::Call(
                    Box::new(M::Builtin(BuiltinFn::Emph, *span)),
                    vec![inner],
                ))
            }
            InlinePiece::Raw(text, span) => Ok(M::Call(
                Box::new(M::Builtin(BuiltinFn::Raw, *span)),
                vec![M::String(Arc::from(text.as_str()))],
            )),
            InlinePiece::Link {
                target,
                label,
                span,
            } => {
                let checked_label = self.check_inline_pieces(label, *span)?;
                Ok(M::Call(
                    Box::new(M::Builtin(BuiltinFn::Link, *span)),
                    vec![M::String(Arc::from(target.as_str())), checked_label],
                ))
            }
            InlinePiece::Image { target, alt, span } => Ok(M::Call(
                Box::new(M::Builtin(BuiltinFn::Image, *span)),
                vec![
                    M::String(Arc::from(target.as_str())),
                    M::String(Arc::from(alt.as_str())),
                ],
            )),
            InlinePiece::Math(text, span) => Ok(M::Call(
                Box::new(M::Builtin(BuiltinFn::Math, *span)),
                vec![M::String(Arc::from(text.as_str()))],
            )),
            InlinePiece::Expr(expr, _) => self.expect_expr(expr, &Type::Inline),
            InlinePiece::Attributed { attrs, target, .. } => {
                let inner = self.check_inline_piece(target)?;
                let lowered = lower_attrs(attrs);
                if lowered.is_empty() {
                    Ok(inner)
                } else {
                    Ok(M::AttachAttrs(Box::new(inner), lowered))
                }
            }
        }
    }

    fn check_block_pieces(
        &mut self,
        pieces: &'a [BlockPiece],
        span: Span,
    ) -> Result<ModuleExpr, (Span, TypeError)> {
        let mut items = Vec::with_capacity(pieces.len());
        for piece in pieces {
            items.push(self.check_block_piece(piece)?);
        }
        Ok(ModuleExpr::BlockFragment(items, Attrs::empty(), span))
    }

    fn check_block_piece(
        &mut self,
        piece: &'a BlockPiece,
    ) -> Result<ModuleExpr, (Span, TypeError)> {
        use ModuleExpr as M;
        let (base, attrs) = match piece {
            BlockPiece::Paragraph {
                inlines,
                attrs,
                span,
            } => {
                let body = self.check_inline_pieces(inlines, *span)?;
                let para = M::Call(Box::new(M::Builtin(BuiltinFn::Para, *span)), vec![body]);
                (para, attrs)
            }
            BlockPiece::Heading {
                level,
                title,
                attrs,
                span,
            } => {
                let body = self.check_inline_pieces(title, *span)?;
                let heading = M::Call(
                    Box::new(M::Builtin(BuiltinFn::Heading, *span)),
                    vec![M::Int(i64::from(*level)), body],
                );
                (heading, attrs)
            }
            BlockPiece::CodeBlock {
                lang,
                text,
                attrs,
                span,
            } => {
                let code = M::Call(
                    Box::new(M::Builtin(BuiltinFn::CodeBlock, *span)),
                    vec![
                        M::String(Arc::from(lang.as_str())),
                        M::String(Arc::from(text.as_str())),
                    ],
                );
                (code, attrs)
            }
            BlockPiece::List {
                ordered,
                items,
                attrs,
                span,
            } => {
                let mut checked_items = Vec::with_capacity(items.len());
                for item in items {
                    checked_items.push(self.check_block_pieces(item, *span)?);
                }
                let lowered = lower_attrs(attrs);
                return Ok(M::BlockList(*ordered, checked_items, lowered, *span));
            }
            BlockPiece::Quote { body, attrs, span } => {
                let inner = self.check_block_pieces(body, *span)?;
                let quote = M::Call(Box::new(M::Builtin(BuiltinFn::Quote, *span)), vec![inner]);
                (quote, attrs)
            }
            BlockPiece::Divider { attrs, span } => {
                let div = M::Call(Box::new(M::Builtin(BuiltinFn::Divider, *span)), vec![]);
                (div, attrs)
            }
            BlockPiece::Table {
                headers,
                rows,
                attrs,
                span,
            } => {
                let mut checked_headers = Vec::with_capacity(headers.len());
                for cell in headers {
                    checked_headers.push(self.check_inline_pieces(cell, *span)?);
                }
                let mut checked_rows = Vec::with_capacity(rows.len());
                for row in rows {
                    let mut checked_row = Vec::with_capacity(row.len());
                    for cell in row {
                        checked_row.push(self.check_inline_pieces(cell, *span)?);
                    }
                    checked_rows.push(checked_row);
                }
                return Ok(M::BlockTable {
                    headers: checked_headers,
                    rows: checked_rows,
                    attrs: lower_attrs(attrs),
                    span: *span,
                });
            }
            BlockPiece::Math { text, attrs, span } => {
                let math = M::Call(
                    Box::new(M::Builtin(BuiltinFn::DisplayMath, *span)),
                    vec![M::String(Arc::from(text.as_str()))],
                );
                (math, attrs)
            }
            BlockPiece::Expr { expr, attrs, span } => {
                let (checked, ty) = self.expression(expr, None)?;
                let block_expr = match ty {
                    Type::Block | Type::Never => checked,
                    Type::Inline => M::Call(
                        Box::new(M::Builtin(BuiltinFn::Para, *span)),
                        vec![checked],
                    ),
                    other => {
                        return Err((
                            expr.span,
                            TypeError::Mismatch {
                                expected: Type::Block,
                                actual: other,
                            },
                        ));
                    }
                };
                (block_expr, attrs)
            }
        };
        let lowered = lower_attrs(attrs);
        if lowered.is_empty() {
            Ok(base)
        } else {
            Ok(M::AttachAttrs(Box::new(base), lowered))
        }
    }

    fn check_binary(
        &mut self,
        op: BinaryOp,
        left: &'a Expr,
        right: &'a Expr,
        span: Span,
    ) -> Result<(ModuleExpr, ModuleExpr, Type), (Span, TypeError)> {
        match op {
            BinaryOp::Or | BinaryOp::And => {
                let checked_left = self.expect_expr(left, &Type::Bool)?;
                let checked_right = self.expect_expr(right, &Type::Bool)?;
                Ok((checked_left, checked_right, Type::Bool))
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let (checked_left, left_ty) = self.expression(left, None)?;
                let (checked_right, right_ty) = self.expression(right, None)?;
                if !supports_equality(&left_ty) {
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
                Ok((checked_left, checked_right, Type::Bool))
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let (checked_left, left_ty) = self.expression(left, None)?;
                let (checked_right, right_ty) = self.expression(right, None)?;
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
                Ok((checked_left, checked_right, Type::Bool))
            }
            BinaryOp::Add => {
                let (checked_left, left_ty) = self.expression(left, None)?;
                let (checked_right, right_ty) = self.expression(right, None)?;
                let supported = matches!(
                    left_ty,
                    Type::Int
                        | Type::Float
                        | Type::String
                        | Type::Inline
                        | Type::Block
                        | Type::List(_)
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
                Ok((checked_left, checked_right, left_ty))
            }
            BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                let (checked_left, left_ty) = self.expression(left, None)?;
                let (checked_right, right_ty) = self.expression(right, None)?;
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
                Ok((checked_left, checked_right, left_ty))
            }
        }
    }

    fn expect_expr(
        &mut self,
        expr: &'a Expr,
        expected: &Type,
    ) -> Result<ModuleExpr, (Span, TypeError)> {
        let resolved = self.resolve_ty(expected);
        let (checked, actual) = self.expression(expr, Some(&resolved))?;
        expect(&resolved, actual, expr.span)?;
        Ok(checked)
    }
}

fn lower_attrs(entries: &[AttrEntry]) -> Attrs {
    if entries.is_empty() {
        return Attrs::empty();
    }
    let pairs = entries
        .iter()
        .map(|entry| {
            let value = match &entry.value {
                AttrLiteral::Bool(b) => AttrValue::Bool(*b),
                AttrLiteral::Int(n) => AttrValue::Int(*n),
                AttrLiteral::Float(f) => AttrValue::Float(*f),
                AttrLiteral::String(s) => AttrValue::String(Arc::from(s.as_str())),
            };
            (Arc::from(entry.key.as_str()), value)
        })
        .collect();
    Attrs::from_entries(pairs)
}

fn supports_equality(ty: &Type) -> bool {
    match ty {
        Type::Unit
        | Type::Bool
        | Type::Int
        | Type::Float
        | Type::String
        | Type::Inline
        | Type::Block
        | Type::Json => true,
        Type::List(elem) => supports_equality(elem),
        Type::Never | Type::Param(_) | Type::Function(_, _) => false,
    }
}

fn expect(expected: &Type, actual: Type, span: Span) -> Result<(), (Span, TypeError)> {
    if actual == Type::Never || *expected == actual {
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
