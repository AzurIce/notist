//! Multi-file module graph, static import binding, per-module checking, and
//! entry-reachability specialization/lowering. Pure in-memory inputs; no filesystem IO.
//!
//! Architecture deliberately separates three layers so future tooling queries
//! and body-level incremental reuse plug in without redesign:
//! 1. Per-file parsing (`parse`) and body-independent interface extraction;
//! 2. Topological import resolution and per-module body checking (`check_module_bodies`);
//! 3. Reachability-driven specialization & lowering to a dense executable `CheckedProgram`.

use crate::ast::{DOC_ENTRY, Decl, Import, ImportKind, Module, Span, Type};
use crate::check::{
    Binding, CheckedDecl, CheckedExpr, CheckedFunction, CheckedProgram, FunctionSig, InstanceKey,
    InterfaceView, MethodSig, ModuleId, ModuleScope, SharedLowered, TypeError, check_module_bodies,
    collect_module_interface, specialize_document_against_shared, specialize_roots,
    specialize_shared_modules,
};
use crate::content::Block;
use crate::eval::{ExecutionError, Value, eval_expr_with_shared};
use crate::lexer::ParseDiagnostic;
use crate::parser::parse_file;
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq)]
pub enum DocumentError {
    Diagnostics(Vec<ProjectDiagnostic>),
    Execution(ExecutionError),
}

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

/// A statically checked and specialized standalone document (`.not` or `.md`)
/// that borrows pre-lowered shared functions from its parent `CheckedProject`
/// without duplicating them. Safe to produce in parallel and evaluate later.
#[derive(Debug)]
pub struct CheckedDocument {
    doc_path: String,
    entry_body: CheckedExpr,
    extra_functions: Vec<CheckedFunction>,
}

impl CheckedDocument {
    /// Normalized virtual path of this checked document.
    pub fn path(&self) -> &str {
        &self.doc_path
    }

    /// Number of document-local or newly instantiated generic functions owned
    /// exclusively by this document (0 when only pre-lowered shared functions
    /// are called).
    pub fn extra_function_count(&self) -> usize {
        self.extra_functions.len()
    }
}

/// Checked multi-module semantic result. All modules and declarations have been
/// statically checked, and monomorphic shared declarations have been pre-lowered
/// once so standalone documents can reference them by `FunctionId` without
/// duplication.
///
/// Immutable and `Sync`, so any number of standalone `.not`/`.md` documents can
/// be checked and evaluated against it in parallel without cloning shared modules.
#[derive(Debug)]
pub struct CheckedProject {
    module_paths: Vec<String>,
    module_ids: HashMap<String, ModuleId>,
    exports: Vec<HashMap<String, Binding>>,
    signatures: Vec<Vec<FunctionSig>>,
    local_methods: Vec<HashMap<(Type, String), MethodSig>>,
    decls: Vec<Vec<CheckedDecl>>,
    shared_lowered: SharedLowered,
}

impl CheckedProject {
    /// Specialize and lower only functions and methods reachable from
    /// `(module_path, entry)` into a compact executable `CheckedProgram`.
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
        if !self.decls[root_id.module.0][root_id.index]
            .type_params
            .is_empty()
        {
            return Err(ExecutionError::UnknownEntry(entry.into()));
        }

        let roots = [(
            entry.to_owned(),
            InstanceKey {
                decl: root_id,
                type_args: Vec::new(),
            },
        )];
        specialize_roots(&self.decls, &roots)
            .map_err(|_| ExecutionError::UnknownEntry(entry.into()))
    }

    /// Lower the reachable subgraph for `(module_path, entry)` and execute it.
    pub fn run(&self, module_path: &str, entry: &str) -> Result<Value, ExecutionError> {
        self.lower_entry(module_path, entry)?.run(entry)
    }

    /// Lower and execute `(module_path, entry)` with a read-only external `DataHost`.
    pub fn run_with_host(
        &self,
        module_path: &str,
        entry: &str,
        host: &dyn crate::json::DataHost,
    ) -> Result<Value, ExecutionError> {
        let normalized = normalize_path(module_path)
            .ok_or_else(|| ExecutionError::UnknownEntry(format!("{module_path}::{entry}")))?;
        self.lower_entry(module_path, entry)?
            .run_with_host(entry, &normalized, host)
    }

    /// Evaluate a `.not` document entry in this checked project and return its root `Block`.
    pub fn eval_document(&self, document_path: &str) -> Result<Block, ExecutionError> {
        match self.run(document_path, DOC_ENTRY)? {
            Value::Block(block) => Ok(block),
            _ => Err(ExecutionError::UnknownEntry(format!(
                "{document_path}::{DOC_ENTRY}"
            ))),
        }
    }

    /// Evaluate a `.not` document entry with a read-only external `DataHost`.
    pub fn eval_document_with_host(
        &self,
        document_path: &str,
        host: &dyn crate::json::DataHost,
    ) -> Result<Block, ExecutionError> {
        match self.run_with_host(document_path, DOC_ENTRY, host)? {
            Value::Block(block) => Ok(block),
            _ => Err(ExecutionError::UnknownEntry(format!(
                "{document_path}::{DOC_ENTRY}"
            ))),
        }
    }

    /// Statically check and specialize a standalone `.not` or `.md` document
    /// against this immutable pre-checked shared `.not` project without
    /// evaluating it or performing any `DataHost` IO.
    ///
    /// Safe to call concurrently from many threads. Pre-lowered shared functions
    /// are referenced by `FunctionId` without cloning.
    pub fn check_standalone_document(
        &self,
        doc_path: &str,
        doc_module: &Module,
    ) -> Result<CheckedDocument, Vec<ProjectDiagnostic>> {
        let doc_path_norm = normalize_path(doc_path).unwrap_or_else(|| doc_path.to_owned());
        let doc_id = ModuleId(self.module_paths.len());
        let mut diagnostics = Vec::new();

        let interface = collect_module_interface(doc_id, doc_module, true);
        for diag in interface.diagnostics {
            diagnostics.push(ProjectDiagnostic {
                module: doc_path_norm.clone(),
                kind: ProjectDiagnosticKind::Type {
                    declaration: diag.declaration,
                    span: diag.span,
                    error: diag.error,
                },
            });
        }

        let mut bindings = interface.bindings;
        let mut methods = interface.methods.clone();
        let mut imported_method_sources: HashMap<(Type, String), ModuleId> = HashMap::new();

        for decl in &doc_module.declarations {
            let Decl::Import(import) = decl else {
                continue;
            };
            let target_id = match resolve_import_path(&doc_path_norm, &import.path) {
                Ok(target_path) => match self.module_ids.get(&target_path) {
                    Some(&id) => id,
                    None => {
                        diagnostics.push(ProjectDiagnostic {
                            module: doc_path_norm.clone(),
                            kind: ProjectDiagnosticKind::Type {
                                declaration: format!("import \"{}\"", import.path),
                                span: import.path_span,
                                error: TypeError::MissingModule(target_path),
                            },
                        });
                        continue;
                    }
                },
                Err(error) => {
                    diagnostics.push(ProjectDiagnostic {
                        module: doc_path_norm.clone(),
                        kind: ProjectDiagnosticKind::Type {
                            declaration: format!("import \"{}\"", import.path),
                            span: import.path_span,
                            error,
                        },
                    });
                    continue;
                }
            };

            let target_exports = &self.exports[target_id.0];
            match &import.kind {
                ImportKind::Default => match default_import_alias(&import.path) {
                    Ok(alias) => insert_binding(
                        &mut bindings,
                        alias,
                        Binding::Namespace(target_id),
                        import.path_span,
                        &doc_path_norm,
                        import,
                        &mut diagnostics,
                    ),
                    Err(error) => diagnostics.push(ProjectDiagnostic {
                        module: doc_path_norm.clone(),
                        kind: ProjectDiagnosticKind::Type {
                            declaration: format!("import \"{}\"", import.path),
                            span: import.path_span,
                            error,
                        },
                    }),
                },
                ImportKind::Alias { alias, span } => insert_binding(
                    &mut bindings,
                    alias.clone(),
                    Binding::Namespace(target_id),
                    *span,
                    &doc_path_norm,
                    import,
                    &mut diagnostics,
                ),
                ImportKind::Members(members) => {
                    for member in members {
                        match target_exports.get(&member.name) {
                            Some(&binding) => insert_binding(
                                &mut bindings,
                                member.alias.clone(),
                                binding,
                                member.alias_span,
                                &doc_path_norm,
                                import,
                                &mut diagnostics,
                            ),
                            None => diagnostics.push(ProjectDiagnostic {
                                module: doc_path_norm.clone(),
                                kind: ProjectDiagnosticKind::Type {
                                    declaration: format!("import \"{}\"", import.path),
                                    span: member.name_span,
                                    error: TypeError::UnknownExport {
                                        module: self.module_paths[target_id.0].clone(),
                                        name: member.name.clone(),
                                    },
                                },
                            }),
                        }
                    }
                }
                ImportKind::Wildcard { span } => {
                    let mut entries: Vec<(&String, &Binding)> = target_exports
                        .iter()
                        .filter(|(name, _)| name.as_str() != DOC_ENTRY)
                        .collect();
                    entries.sort_by_key(|(a, _)| *a);
                    for (name, &binding) in entries {
                        insert_binding(
                            &mut bindings,
                            name.clone(),
                            binding,
                            *span,
                            &doc_path_norm,
                            import,
                            &mut diagnostics,
                        );
                    }
                }
            }

            let mut target_methods: Vec<(&(Type, String), &MethodSig)> =
                self.local_methods[target_id.0].iter().collect();
            target_methods.sort_by(|((_, a), _), ((_, b), _)| a.cmp(b));
            for (key, sig) in target_methods {
                if interface.methods.contains_key(key) {
                    continue;
                }
                if let Some(&prev_module) = imported_method_sources.get(key) {
                    if prev_module != target_id {
                        diagnostics.push(ProjectDiagnostic {
                            module: doc_path_norm.clone(),
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

        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }

        let scope = ModuleScope { bindings, methods };
        let view = InterfaceView {
            module_paths: &self.module_paths,
            exports: &self.exports,
            signatures: &self.signatures,
            extra_signatures: Some(&interface.signatures),
        };
        let (doc_decls, body_diags) =
            check_module_bodies(&doc_path_norm, doc_module, &scope, &view);
        if !body_diags.is_empty() {
            for diag in body_diags {
                diagnostics.push(ProjectDiagnostic {
                    module: doc_path_norm.clone(),
                    kind: ProjectDiagnosticKind::Type {
                        declaration: diag.declaration,
                        span: diag.span,
                        error: diag.error,
                    },
                });
            }
            return Err(diagnostics);
        }

        let Some(&Binding::Function(root_id)) = scope.bindings.get(DOC_ENTRY) else {
            diagnostics.push(ProjectDiagnostic {
                module: doc_path_norm,
                kind: ProjectDiagnosticKind::Type {
                    declaration: DOC_ENTRY.to_owned(),
                    span: Span::DUMMY,
                    error: TypeError::UnknownName(DOC_ENTRY.to_owned()),
                },
            });
            return Err(diagnostics);
        };
        let root_key = InstanceKey {
            decl: root_id,
            type_args: Vec::new(),
        };
        let lowered = specialize_document_against_shared(
            &self.decls,
            &self.shared_lowered,
            &doc_decls,
            root_key,
        )
        .map_err(|errs| {
            errs.into_iter()
                .map(|(module_id, diag)| {
                    let module = if module_id == doc_id {
                        doc_path_norm.clone()
                    } else {
                        self.module_paths[module_id.0].clone()
                    };
                    ProjectDiagnostic {
                        module,
                        kind: ProjectDiagnosticKind::Type {
                            declaration: diag.declaration,
                            span: diag.span,
                            error: diag.error,
                        },
                    }
                })
                .collect::<Vec<_>>()
        })?;

        Ok(CheckedDocument {
            doc_path: doc_path_norm,
            entry_body: lowered.entry_body,
            extra_functions: lowered.extra_functions,
        })
    }

    /// Evaluate a previously checked standalone document against this project's
    /// pre-lowered shared functions and a read-only external `DataHost`.
    /// Safe to call concurrently from many threads.
    pub fn eval_checked_document(
        &self,
        doc: &CheckedDocument,
        host: &dyn crate::json::DataHost,
    ) -> Result<Block, ExecutionError> {
        match eval_expr_with_shared(
            &self.shared_lowered.functions,
            &doc.extra_functions,
            &doc.entry_body,
            &doc.doc_path,
            host,
        )? {
            Value::Block(block) => Ok(block),
            _ => Err(ExecutionError::UnknownEntry(format!(
                "{}::{DOC_ENTRY}",
                doc.doc_path
            ))),
        }
    }

    /// Convenience helper combining `check_standalone_document` and
    /// `eval_checked_document`.
    pub fn check_and_eval_standalone_document(
        &self,
        doc_path: &str,
        doc_module: &Module,
        host: &dyn crate::json::DataHost,
    ) -> Result<Block, DocumentError> {
        let checked = self
            .check_standalone_document(doc_path, doc_module)
            .map_err(DocumentError::Diagnostics)?;
        self.eval_checked_document(&checked, host)
            .map_err(DocumentError::Execution)
    }
}

/// Parse and statically check an explicit in-memory set of `(path, source)`
/// files (`.not` modules and Markdown documents). Relative `import` paths are
/// resolved lexically against each file's normalized path.
pub fn check_project(files: &[(&str, &str)]) -> Result<CheckedProject, Vec<ProjectDiagnostic>> {
    let mut parsed_modules = Vec::with_capacity(files.len());
    let mut diagnostics = Vec::new();

    for &(raw_path, source) in files {
        let path = normalize_path(raw_path).unwrap_or_else(|| raw_path.to_owned());
        let parsed = parse_file(raw_path, source);
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
    let mut signatures: Vec<Vec<FunctionSig>> = Vec::with_capacity(modules.len());
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
                    let mut entries: Vec<(&String, &Binding)> = target_exports
                        .iter()
                        .filter(|(name, _)| name.as_str() != DOC_ENTRY)
                        .collect();
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
        extra_signatures: None,
    };
    let mut decls = Vec::with_capacity(modules.len());
    for (index, &(_, module)) in modules.iter().enumerate() {
        let scope = scopes[index]
            .as_ref()
            .expect("topological pass initialized every module scope");
        let (checked_decls, body_diags) =
            check_module_bodies(&module_paths[index], module, scope, &view);
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

    // Pre-lower all concrete monomorphic shared declarations (and everything they
    // transitively call) once, validating that no reachable specialization hits
    // `compile_error(...)`. Standalone documents reuse this table by FunctionId.
    let mut mono_roots = Vec::new();
    for (m_idx, mod_decls) in decls.iter().enumerate() {
        for (d_idx, decl) in mod_decls.iter().enumerate() {
            if decl.type_params.is_empty() {
                mono_roots.push(InstanceKey {
                    decl: crate::check::DeclId {
                        module: ModuleId(m_idx),
                        index: d_idx,
                    },
                    type_args: Vec::new(),
                });
            }
        }
    }
    let shared_lowered = match specialize_shared_modules(&decls, &mono_roots) {
        Ok(lowered) => lowered,
        Err(spec_errors) => {
            for (module_id, diag) in spec_errors {
                diagnostics.push(ProjectDiagnostic {
                    module: module_paths[module_id.0].clone(),
                    kind: ProjectDiagnosticKind::Type {
                        declaration: diag.declaration,
                        span: diag.span,
                        error: diag.error,
                    },
                });
            }
            return Err(diagnostics);
        }
    };

    Ok(CheckedProject {
        module_paths,
        module_ids,
        exports,
        signatures,
        local_methods,
        decls,
        shared_lowered,
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

pub(crate) fn default_import_alias(import_path: &str) -> Result<String, TypeError> {
    let filename = import_path.rsplit('/').next().unwrap_or(import_path);
    let stem = filename
        .strip_suffix(".not")
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
        "import" | "as" | "fn" | "where" | "match" | "let" | "if" | "else" | "true" | "false"
    )
}
