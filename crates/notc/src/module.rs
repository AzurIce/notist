//! Multi-file module graph, static import binding, per-module checking, and
//! entry-reachability lowering. Pure in-memory inputs; no filesystem IO.
//!
//! Architecture deliberately separates three layers so future tooling queries
//! and body-level incremental reuse plug in without redesign:
//! 1. Per-file parsing (`parse`) and body-independent interface extraction;
//! 2. Topological import resolution and per-module body checking (`check_module_bodies`);
//! 3. Reachability-driven lowering to a dense executable `CheckedProgram`.

use crate::ast::{Decl, Import, ImportKind, Module, Span, Type};
use crate::check::{
    Binding, CheckedDecl, CheckedExpr, CheckedFunction, CheckedProgram, DeclId, FunctionId,
    InterfaceView, MethodSig, ModuleExpr, ModuleId, ModuleScope, TypeError, check_module_bodies,
    collect_module_interface,
};
use crate::eval::{ExecutionError, Value};
use crate::lexer::ParseDiagnostic;
use crate::parser::parse;
use std::collections::{HashMap, VecDeque};

#[derive(Debug, PartialEq, Eq)]
pub struct ProjectDiagnostic {
    pub module: String,
    pub kind: ProjectDiagnosticKind,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProjectDiagnosticKind {
    Parse(ParseDiagnostic),
    Type {
        declaration: String,
        span: Span,
        error: TypeError,
    },
}

/// Checked multi-module semantic result. All modules and declarations have been
/// statically checked; execution lowers only the subgraph reachable from a
/// chosen `(module, entry)`.
#[derive(Debug)]
pub struct CheckedProject {
    module_ids: HashMap<String, ModuleId>,
    exports: Vec<HashMap<String, Binding>>,
    decls: Vec<Vec<CheckedDecl>>,
}

impl CheckedProject {
    /// Lower only functions and methods reachable from `(module_path, entry)`
    /// into a compact executable `CheckedProgram`.
    pub fn lower_entry(
        &self,
        module_path: &str,
        entry: &str,
    ) -> Result<CheckedProgram, ExecutionError> {
        let normalized = normalize_path(module_path)
            .ok_or_else(|| ExecutionError::UnknownEntry(format!("{module_path}::{entry}")))?;
        let Some(&module_id) = self.module_ids.get(&normalized) else {
            return Err(ExecutionError::UnknownEntry(format!(
                "{module_path}::{entry}"
            )));
        };
        let Some(&Binding::Function(root_id)) = self.exports[module_id.0].get(entry) else {
            return Err(ExecutionError::UnknownEntry(entry.into()));
        };

        let mut mapping: HashMap<DeclId, FunctionId> = HashMap::new();
        let mut order: Vec<DeclId> = Vec::new();
        let mut queue: VecDeque<DeclId> = VecDeque::new();

        mapping.insert(root_id, FunctionId(0));
        order.push(root_id);
        queue.push_back(root_id);

        while let Some(decl_id) = queue.pop_front() {
            let decl = &self.decls[decl_id.module.0][decl_id.index];
            collect_referenced_decls(&decl.body, &mut |target| {
                if let std::collections::hash_map::Entry::Vacant(entry) = mapping.entry(target) {
                    let next = FunctionId(order.len());
                    entry.insert(next);
                    order.push(target);
                    queue.push_back(target);
                }
            });
        }

        let functions = order
            .into_iter()
            .map(|decl_id| {
                let decl = &self.decls[decl_id.module.0][decl_id.index];
                CheckedFunction {
                    arity: decl.arity,
                    body: remap_expr(&decl.body, &mapping),
                }
            })
            .collect();

        let mut function_names = HashMap::with_capacity(1);
        function_names.insert(entry.to_owned(), FunctionId(0));

        Ok(CheckedProgram {
            function_names,
            functions,
        })
    }

    /// Lower the reachable subgraph for `(module_path, entry)` and execute it.
    pub fn run(&self, module_path: &str, entry: &str) -> Result<Value, ExecutionError> {
        self.lower_entry(module_path, entry)?.run(entry)
    }
}

/// Parse and statically check an explicit in-memory set of `(path, source)`
/// modules. Relative `import` paths are resolved lexically against each
/// module's normalized path.
pub fn check_project(files: &[(&str, &str)]) -> Result<CheckedProject, Vec<ProjectDiagnostic>> {
    let mut parsed_modules = Vec::with_capacity(files.len());
    let mut diagnostics = Vec::new();

    for &(raw_path, source) in files {
        let path = normalize_path(raw_path).unwrap_or_else(|| raw_path.to_owned());
        let parsed = parse(source);
        for diag in parsed.diagnostics {
            diagnostics.push(ProjectDiagnostic {
                module: path.clone(),
                kind: ProjectDiagnosticKind::Parse(diag),
            });
        }
        parsed_modules.push((path, parsed.program.root));
    }

    let borrowed: Vec<(&str, &Module)> = parsed_modules
        .iter()
        .map(|(path, module)| (path.as_str(), module))
        .collect();
    match check_parsed_modules(&borrowed) {
        Ok(project) if diagnostics.is_empty() => Ok(project),
        Ok(_) => Err(diagnostics),
        Err(type_diags) => {
            diagnostics.extend(type_diags);
            Err(diagnostics)
        }
    }
}

/// Statically check a set of already-parsed modules (entry point designed for
/// future per-file parse caching).
pub fn check_parsed_modules(
    modules: &[(&str, &Module)],
) -> Result<CheckedProject, Vec<ProjectDiagnostic>> {
    let mut module_paths = Vec::with_capacity(modules.len());
    let mut module_ids = HashMap::with_capacity(modules.len());
    let mut diagnostics = Vec::new();

    for (index, &(raw_path, _)) in modules.iter().enumerate() {
        let path = normalize_path(raw_path).unwrap_or_else(|| raw_path.to_owned());
        if module_ids.insert(path.clone(), ModuleId(index)).is_some() {
            diagnostics.push(ProjectDiagnostic {
                module: path.clone(),
                kind: ProjectDiagnosticKind::Type {
                    declaration: path.clone(),
                    span: Span::DUMMY,
                    error: TypeError::DuplicateBinding(path.clone()),
                },
            });
        }
        module_paths.push(path);
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    // Layer 1a: per-module local declarations & resolved import edges (no body inspection).
    let mut signatures: Vec<Vec<Type>> = Vec::with_capacity(modules.len());
    let mut local_bindings: Vec<HashMap<String, Binding>> = Vec::with_capacity(modules.len());
    let mut local_methods: Vec<HashMap<(Type, String), MethodSig>> =
        Vec::with_capacity(modules.len());
    let mut resolved_imports: Vec<Vec<(&Import, ModuleId)>> = Vec::with_capacity(modules.len());

    for (index, &(_, module)) in modules.iter().enumerate() {
        let module_id = ModuleId(index);
        let interface = collect_module_interface(module_id, module, true);
        for diag in interface.diagnostics {
            diagnostics.push(ProjectDiagnostic {
                module: module_paths[index].clone(),
                kind: ProjectDiagnosticKind::Type {
                    declaration: diag.declaration,
                    span: diag.span,
                    error: diag.error,
                },
            });
        }
        signatures.push(interface.signatures);
        local_bindings.push(interface.bindings);
        local_methods.push(interface.methods);

        let mut edges = Vec::new();
        for decl in &module.declarations {
            if let Decl::Import(import) = decl {
                match resolve_import_path(&module_paths[index], &import.path) {
                    Ok(target_path) => match module_ids.get(&target_path) {
                        Some(&target_id) => edges.push((import, target_id)),
                        None => diagnostics.push(ProjectDiagnostic {
                            module: module_paths[index].clone(),
                            kind: ProjectDiagnosticKind::Type {
                                declaration: format!("import \"{}\"", import.path),
                                span: import.path_span,
                                error: TypeError::MissingModule(target_path),
                            },
                        }),
                    },
                    Err(error) => diagnostics.push(ProjectDiagnostic {
                        module: module_paths[index].clone(),
                        kind: ProjectDiagnosticKind::Type {
                            declaration: format!("import \"{}\"", import.path),
                            span: import.path_span,
                            error,
                        },
                    }),
                }
            }
        }
        resolved_imports.push(edges);
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    // Layer 1b: static import DAG topological order & deterministic cycle detection.
    let topo_order = topological_order(&module_paths, &resolved_imports)?;

    // Layer 1c: compute per-module ExportTable and ModuleScope in topological order.
    let mut exports: Vec<HashMap<String, Binding>> = vec![HashMap::new(); modules.len()];
    let mut scopes: Vec<Option<ModuleScope>> = (0..modules.len()).map(|_| None).collect();

    for module_id in topo_order {
        let idx = module_id.0;
        let mut bindings = std::mem::take(&mut local_bindings[idx]);
        let mut methods = local_methods[idx].clone();
        let mut imported_method_sources: HashMap<(Type, String), ModuleId> = HashMap::new();

        for &(import, target_id) in &resolved_imports[idx] {
            let target_exports = &exports[target_id.0];
            match &import.kind {
                ImportKind::Default => match default_import_alias(&import.path) {
                    Ok(alias) => {
                        insert_binding(
                            &mut bindings,
                            alias,
                            Binding::Namespace(target_id),
                            import.path_span,
                            &module_paths[idx],
                            import,
                            &mut diagnostics,
                        );
                    }
                    Err(error) => diagnostics.push(ProjectDiagnostic {
                        module: module_paths[idx].clone(),
                        kind: ProjectDiagnosticKind::Type {
                            declaration: format!("import \"{}\"", import.path),
                            span: import.path_span,
                            error,
                        },
                    }),
                },
                ImportKind::Alias { alias, span } => {
                    insert_binding(
                        &mut bindings,
                        alias.clone(),
                        Binding::Namespace(target_id),
                        *span,
                        &module_paths[idx],
                        import,
                        &mut diagnostics,
                    );
                }
                ImportKind::Members(members) => {
                    for member in members {
                        match target_exports.get(&member.name) {
                            Some(&binding) => {
                                insert_binding(
                                    &mut bindings,
                                    member.alias.clone(),
                                    binding,
                                    member.alias_span,
                                    &module_paths[idx],
                                    import,
                                    &mut diagnostics,
                                );
                            }
                            None => diagnostics.push(ProjectDiagnostic {
                                module: module_paths[idx].clone(),
                                kind: ProjectDiagnosticKind::Type {
                                    declaration: format!("import \"{}\"", import.path),
                                    span: member.name_span,
                                    error: TypeError::UnknownExport {
                                        module: module_paths[target_id.0].clone(),
                                        name: member.name.clone(),
                                    },
                                },
                            }),
                        }
                    }
                }
                ImportKind::Wildcard { span } => {
                    // Deterministic order so duplicate diagnostics are reproducible.
                    let mut entries: Vec<(&String, &Binding)> = target_exports.iter().collect();
                    entries.sort_by_key(|(a, _)| *a);
                    for (name, &binding) in entries {
                        insert_binding(
                            &mut bindings,
                            name.clone(),
                            binding,
                            *span,
                            &module_paths[idx],
                            import,
                            &mut diagnostics,
                        );
                    }
                }
            }

            // Directly imported modules expose their declared type methods into the
            // importing module's method table (local methods take precedence, while
            // conflicting methods from two distinct imported modules are rejected).
            let mut target_methods: Vec<(&(Type, String), &MethodSig)> =
                local_methods[target_id.0].iter().collect();
            target_methods.sort_by(|((_, a), _), ((_, b), _)| a.cmp(b));
            for (key, sig) in target_methods {
                if local_methods[idx].contains_key(key) {
                    continue;
                }
                if let Some(&prev_module) = imported_method_sources.get(key) {
                    if prev_module != target_id {
                        diagnostics.push(ProjectDiagnostic {
                            module: module_paths[idx].clone(),
                            kind: ProjectDiagnosticKind::Type {
                                declaration: format!("import \"{}\"", import.path),
                                span: import.span,
                                error: TypeError::AmbiguousMethod {
                                    receiver: key.0.clone(),
                                    method: key.1.clone(),
                                },
                            },
                        });
                    }
                    continue;
                }
                imported_method_sources.insert(key.clone(), target_id);
                methods.insert(key.clone(), sig.clone());
            }
        }

        exports[idx] = bindings.clone();
        scopes[idx] = Some(ModuleScope { bindings, methods });
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    // Layer 2: check every module's function/method bodies independently.
    let view = InterfaceView {
        module_paths: &module_paths,
        exports: &exports,
        signatures: &signatures,
    };
    let mut decls = Vec::with_capacity(modules.len());
    for (index, &(_, module)) in modules.iter().enumerate() {
        let scope = scopes[index]
            .as_ref()
            .expect("topological pass initialized every module scope");
        let (checked_decls, body_diags) = check_module_bodies(module, scope, &view);
        for diag in body_diags {
            diagnostics.push(ProjectDiagnostic {
                module: module_paths[index].clone(),
                kind: ProjectDiagnosticKind::Type {
                    declaration: diag.declaration,
                    span: diag.span,
                    error: diag.error,
                },
            });
        }
        decls.push(checked_decls);
    }

    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }

    Ok(CheckedProject {
        module_ids,
        exports,
        decls,
    })
}

fn insert_binding(
    bindings: &mut HashMap<String, Binding>,
    name: String,
    binding: Binding,
    span: Span,
    module_path: &str,
    import: &Import,
    diagnostics: &mut Vec<ProjectDiagnostic>,
) {
    match bindings.entry(name) {
        std::collections::hash_map::Entry::Occupied(entry) => {
            diagnostics.push(ProjectDiagnostic {
                module: module_path.to_owned(),
                kind: ProjectDiagnosticKind::Type {
                    declaration: format!("import \"{}\"", import.path),
                    span,
                    error: TypeError::DuplicateBinding(entry.key().clone()),
                },
            });
        }
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(binding);
        }
    }
}

fn topological_order(
    module_paths: &[String],
    edges: &[Vec<(&Import, ModuleId)>],
) -> Result<Vec<ModuleId>, Vec<ProjectDiagnostic>> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Unvisited,
        Visiting,
        Done,
    }

    let mut state = vec![State::Unvisited; module_paths.len()];
    let mut stack: Vec<ModuleId> = Vec::new();
    let mut order = Vec::with_capacity(module_paths.len());
    let mut diagnostics = Vec::new();

    #[allow(clippy::too_many_arguments)]
    fn visit(
        id: ModuleId,
        module_paths: &[String],
        edges: &[Vec<(&Import, ModuleId)>],
        state: &mut [State],
        stack: &mut Vec<ModuleId>,
        order: &mut Vec<ModuleId>,
        diagnostics: &mut Vec<ProjectDiagnostic>,
    ) {
        state[id.0] = State::Visiting;
        stack.push(id);

        for &(import, target) in &edges[id.0] {
            match state[target.0] {
                State::Unvisited => visit(
                    target,
                    module_paths,
                    edges,
                    state,
                    stack,
                    order,
                    diagnostics,
                ),
                State::Visiting => {
                    let start = stack
                        .iter()
                        .position(|&m| m == target)
                        .expect("visiting node is on DFS stack");
                    let mut cycle: Vec<String> = stack[start..]
                        .iter()
                        .map(|m| module_paths[m.0].clone())
                        .collect();
                    cycle.push(module_paths[target.0].clone());
                    diagnostics.push(ProjectDiagnostic {
                        module: module_paths[id.0].clone(),
                        kind: ProjectDiagnosticKind::Type {
                            declaration: format!("import \"{}\"", import.path),
                            span: import.path_span,
                            error: TypeError::ImportCycle(cycle),
                        },
                    });
                }
                State::Done => {}
            }
        }

        stack.pop();
        state[id.0] = State::Done;
        order.push(id);
    }

    for i in 0..module_paths.len() {
        if state[i] == State::Unvisited {
            visit(
                ModuleId(i),
                module_paths,
                edges,
                &mut state,
                &mut stack,
                &mut order,
                &mut diagnostics,
            );
        }
    }

    if diagnostics.is_empty() {
        Ok(order)
    } else {
        Err(diagnostics)
    }
}

/// Normalize a relative POSIX-style virtual module path (collapsing `.` and `..`).
/// Returns `None` for empty paths, absolute paths, or paths escaping the root.
pub fn normalize_path(path: &str) -> Option<String> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

/// Resolve a relative import path against a normalized importing module path.
pub fn resolve_import_path(from_module: &str, import_path: &str) -> Result<String, TypeError> {
    if import_path.is_empty() || import_path.starts_with('/') || import_path.contains('\\') {
        return Err(TypeError::InvalidImportPath(import_path.to_owned()));
    }
    let base_dir = match from_module.rfind('/') {
        Some(pos) => &from_module[..pos],
        None => "",
    };
    let joined = if base_dir.is_empty() {
        import_path.to_owned()
    } else {
        format!("{base_dir}/{import_path}")
    };
    normalize_path(&joined).ok_or_else(|| TypeError::InvalidImportPath(import_path.to_owned()))
}

fn default_import_alias(import_path: &str) -> Result<String, TypeError> {
    let filename = import_path.rsplit('/').next().unwrap_or(import_path);
    let stem = filename
        .strip_suffix(".notc")
        .or_else(|| filename.rsplit_once('.').map(|(s, _)| s))
        .unwrap_or(filename);
    if is_valid_binding_ident(stem) {
        Ok(stem.to_owned())
    } else {
        Err(TypeError::InvalidImportAlias(stem.to_owned()))
    }
}

fn is_valid_binding_ident(name: &str) -> bool {
    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    if !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return false;
    }
    !matches!(
        name,
        "import" | "as" | "fn" | "let" | "if" | "else" | "true" | "false"
    )
}

fn collect_referenced_decls(expr: &ModuleExpr, visit: &mut impl FnMut(DeclId)) {
    use ModuleExpr as M;
    match expr {
        M::Int(_) | M::Bool(_) | M::Local(_) => {}
        M::Function(id) => visit(*id),
        M::Let(init, body) => {
            collect_referenced_decls(init, visit);
            collect_referenced_decls(body, visit);
        }
        M::Add(left, right) => {
            collect_referenced_decls(left, visit);
            collect_referenced_decls(right, visit);
        }
        M::If(cond, yes, no) => {
            collect_referenced_decls(cond, visit);
            collect_referenced_decls(yes, visit);
            collect_referenced_decls(no, visit);
        }
        M::Call(callee, args) => {
            collect_referenced_decls(callee, visit);
            for arg in args {
                collect_referenced_decls(arg, visit);
            }
        }
    }
}

fn remap_expr(expr: &ModuleExpr, mapping: &HashMap<DeclId, FunctionId>) -> CheckedExpr {
    use CheckedExpr as C;
    use ModuleExpr as M;
    match expr {
        M::Int(n) => C::Int(*n),
        M::Bool(b) => C::Bool(*b),
        M::Local(slot) => C::Local(*slot),
        M::Function(id) => C::Function(mapping[id]),
        M::Let(init, body) => C::Let(
            Box::new(remap_expr(init, mapping)),
            Box::new(remap_expr(body, mapping)),
        ),
        M::Add(left, right) => C::Add(
            Box::new(remap_expr(left, mapping)),
            Box::new(remap_expr(right, mapping)),
        ),
        M::If(cond, yes, no) => C::If(
            Box::new(remap_expr(cond, mapping)),
            Box::new(remap_expr(yes, mapping)),
            Box::new(remap_expr(no, mapping)),
        ),
        M::Call(callee, args) => C::Call(
            Box::new(remap_expr(callee, mapping)),
            args.iter().map(|a| remap_expr(a, mapping)).collect(),
        ),
    }
}
