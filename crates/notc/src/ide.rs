//! Pure in-memory IDE / LSP semantic analysis layer for `.notc` modules and
//! `.not` documents.
//!
//! Designed to be error-tolerant: even when files contain syntax errors,
//! missing imports, or partial expressions while typing, all valid symbols,
//! hovers, go-to-definition targets, and type-aware completions remain queryable.

use crate::ast::{
    AttrLiteral, BlockPiece, DOC_ENTRY, Decl, Expr, ExprKind, Function, Import, ImportKind,
    InlinePiece, Method, Module, Span, Type, TypePattern, WhereExpr, WhereExprKind,
};
use crate::builtin::{
    all_prelude_functions, builtin_methods_for_type, lookup_builtin_method,
    lookup_prelude_function,
};
use crate::check::{
    Binding, DeclId, FunctionSig, InstanceKey, InterfaceView, MethodSig, ModuleId, ModuleScope,
    TypeError, check_module_bodies, collect_module_interface, format_type, format_type_error,
    specialize_roots, substitute_type,
};
use crate::module::{default_import_alias, normalize_path, resolve_import_path};
use crate::parser::parse_file_with_frontends;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticPhase {
    Parse,
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdeDiagnostic {
    pub module: String,
    pub span: Span,
    pub message: String,
    pub phase: DiagnosticPhase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Method,
    Heading,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentSymbol {
    pub name: String,
    pub detail: Option<String>,
    pub kind: SymbolKind,
    pub span: Span,
    pub selection_span: Span,
    pub children: Vec<DocumentSymbol>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverInfo {
    pub span: Span,
    pub contents: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefinitionTarget {
    pub module: String,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionKind {
    Function,
    Method,
    Variable,
    Module,
    Keyword,
    Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionCandidate {
    pub label: String,
    pub kind: CompletionKind,
    pub detail: Option<String>,
    pub replace_span: Span,
}

#[derive(Debug, Clone)]
struct DeclLocation {
    module: String,
    name_span: Span,
    signature: String,
}

#[derive(Debug, Clone)]
struct DefinitionLink {
    origin_span: Span,
    target_module: String,
    target_span: Span,
}

#[derive(Debug, Clone)]
struct LocalScopeEntry {
    scope_span: Span,
    name: String,
    ty: Type,
}

#[derive(Debug, Clone)]
struct ModuleIndex {
    source: String,
    is_document: bool,
    symbols: Vec<DocumentSymbol>,
    hovers: Vec<HoverInfo>,
    definitions: Vec<DefinitionLink>,
    local_scopes: Vec<LocalScopeEntry>,
    expr_types: Vec<(Span, Type)>,
    scope: ModuleScope,
}

#[derive(Debug)]
pub struct WorkspaceAnalysis {
    diagnostics: Vec<IdeDiagnostic>,
    module_indices: HashMap<String, usize>,
    module_paths: Vec<String>,
    modules: Vec<ModuleIndex>,
    exports: Vec<HashMap<String, Binding>>,
    signatures: Vec<Vec<FunctionSig>>,
    decl_locations: Vec<Vec<DeclLocation>>,
}

impl WorkspaceAnalysis {
    /// Analyze an explicit in-memory set of `(path, source)` files (`.notc` and `.not`).
    pub fn analyze(files: &[(&str, &str)]) -> Self {
        Self::analyze_with_frontends(files, &[])
    }

    /// Analyze an explicit in-memory set of `(path, source)` files with pluggable
    /// [`DocumentFrontend`](crate::DocumentFrontend)s (e.g. Markdown).
    pub fn analyze_with_frontends(
        files: &[(&str, &str)],
        frontends: &[&dyn crate::frontend::DocumentFrontend],
    ) -> Self {
        let mut diagnostics: Vec<IdeDiagnostic> = Vec::new();
        let mut module_paths: Vec<String> = Vec::with_capacity(files.len());
        let mut sources: Vec<String> = Vec::with_capacity(files.len());
        let mut parsed_modules: Vec<Module> = Vec::with_capacity(files.len());
        let mut module_indices: HashMap<String, usize> = HashMap::with_capacity(files.len());

        for &(raw_path, source) in files {
            let path = normalize_path(raw_path).unwrap_or_else(|| raw_path.to_owned());
            if module_indices.contains_key(&path) {
                diagnostics.push(IdeDiagnostic {
                    module: path.clone(),
                    span: Span::DUMMY,
                    message: format!("duplicate file path `{path}`"),
                    phase: DiagnosticPhase::Type,
                });
                continue;
            }
            let idx = module_paths.len();
            module_indices.insert(path.clone(), idx);
            let parsed = parse_file_with_frontends(&path, source, frontends);
            for diag in parsed.diagnostics {
                diagnostics.push(IdeDiagnostic {
                    module: path.clone(),
                    span: diag.span,
                    message: diag.message,
                    phase: DiagnosticPhase::Parse,
                });
            }
            module_paths.push(path);
            sources.push(source.to_owned());
            parsed_modules.push(parsed.program.root);
        }

        let count = parsed_modules.len();

        // Layer 1a: collect per-module interfaces, declaration locations, symbols, and doc IDs.
        let mut signatures: Vec<Vec<FunctionSig>> = Vec::with_capacity(count);
        let mut local_bindings: Vec<HashMap<String, Binding>> = Vec::with_capacity(count);
        let mut local_methods: Vec<HashMap<(Type, String), MethodSig>> = Vec::with_capacity(count);
        let mut decl_locations: Vec<Vec<DeclLocation>> = Vec::with_capacity(count);
        let mut resolved_imports: Vec<Vec<(&Import, ModuleId)>> = Vec::with_capacity(count);
        let mut doc_ids: HashMap<String, HashMap<String, Span>> = HashMap::new();
        let mut module_symbols: Vec<Vec<DocumentSymbol>> = Vec::with_capacity(count);

        for idx in 0..count {
            let module_id = ModuleId(idx);
            let module = &parsed_modules[idx];
            let path = &module_paths[idx];
            let is_doc = !path.ends_with(".notc");

            let interface = collect_module_interface(module_id, module, true);
            for diag in interface.diagnostics {
                diagnostics.push(IdeDiagnostic {
                    module: path.clone(),
                    span: diag.span,
                    message: format_type_error(&diag.error),
                    phase: DiagnosticPhase::Type,
                });
            }
            signatures.push(interface.signatures);
            local_bindings.push(interface.bindings);
            local_methods.push(interface.methods);

            let mut locs = Vec::new();
            let mut syms = Vec::new();
            let mut edges = Vec::new();
            let mut ids_for_doc = HashMap::new();

            for decl in &module.declarations {
                match decl {
                    Decl::Import(import) => match resolve_import_path(path, &import.path) {
                        Ok(target_path) => match module_indices.get(&target_path) {
                            Some(&target_idx) => edges.push((import, ModuleId(target_idx))),
                            None => diagnostics.push(IdeDiagnostic {
                                module: path.clone(),
                                span: import.path_span,
                                message: format_type_error(&TypeError::MissingModule(target_path)),
                                phase: DiagnosticPhase::Type,
                            }),
                        },
                        Err(error) => diagnostics.push(IdeDiagnostic {
                            module: path.clone(),
                            span: import.path_span,
                            message: format_type_error(&error),
                            phase: DiagnosticPhase::Type,
                        }),
                    },
                    Decl::Function(func) => {
                        let sig = format_function_signature(func);
                        locs.push(DeclLocation {
                            module: path.clone(),
                            name_span: func.name_span,
                            signature: sig.clone(),
                        });
                        if is_doc && func.name == DOC_ENTRY {
                            if let Some(body) = &func.body {
                                extract_document_outline(body, &mut syms, &mut ids_for_doc);
                            }
                        } else {
                            syms.push(DocumentSymbol {
                                name: func.name.clone(),
                                detail: Some(sig),
                                kind: SymbolKind::Function,
                                span: func.span,
                                selection_span: func.name_span,
                                children: Vec::new(),
                            });
                        }
                    }
                    Decl::Method(method) => {
                        let sig = format_method_signature(method);
                        locs.push(DeclLocation {
                            module: path.clone(),
                            name_span: method.name_span,
                            signature: sig.clone(),
                        });
                        syms.push(DocumentSymbol {
                            name: format!("{}.{}", format_type(&method.receiver_type), method.name),
                            detail: Some(sig),
                            kind: SymbolKind::Method,
                            span: method.span,
                            selection_span: method.name_span,
                            children: Vec::new(),
                        });
                    }
                }
            }

            if is_doc {
                doc_ids.insert(path.clone(), ids_for_doc);
            }
            decl_locations.push(locs);
            module_symbols.push(syms);
            resolved_imports.push(edges);
        }

        // Layer 1b: resilient topological order (records cycle diagnostics, continues).
        let topo_order = resilient_topological_order(
            &module_paths,
            &resolved_imports,
            &mut diagnostics,
        );

        // Layer 1c: compute ExportTable and ModuleScope in topological order.
        let mut exports: Vec<HashMap<String, Binding>> = vec![HashMap::new(); count];
        let mut scopes: Vec<Option<ModuleScope>> = (0..count).map(|_| None).collect();

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
                            insert_ide_binding(
                                &mut bindings,
                                alias,
                                Binding::Namespace(target_id),
                                import.path_span,
                                &module_paths[idx],
                                &mut diagnostics,
                            );
                        }
                        Err(error) => diagnostics.push(IdeDiagnostic {
                            module: module_paths[idx].clone(),
                            span: import.path_span,
                            message: format_type_error(&error),
                            phase: DiagnosticPhase::Type,
                        }),
                    },
                    ImportKind::Alias { alias, span } => {
                        insert_ide_binding(
                            &mut bindings,
                            alias.clone(),
                            Binding::Namespace(target_id),
                            *span,
                            &module_paths[idx],
                            &mut diagnostics,
                        );
                    }
                    ImportKind::Members(members) => {
                        for member in members {
                            match target_exports.get(&member.name) {
                                Some(&binding) => {
                                    insert_ide_binding(
                                        &mut bindings,
                                        member.alias.clone(),
                                        binding,
                                        member.alias_span,
                                        &module_paths[idx],
                                        &mut diagnostics,
                                    );
                                }
                                None => diagnostics.push(IdeDiagnostic {
                                    module: module_paths[idx].clone(),
                                    span: member.name_span,
                                    message: format_type_error(&TypeError::UnknownExport {
                                        module: module_paths[target_id.0].clone(),
                                        name: member.name.clone(),
                                    }),
                                    phase: DiagnosticPhase::Type,
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
                            insert_ide_binding(
                                &mut bindings,
                                name.clone(),
                                binding,
                                *span,
                                &module_paths[idx],
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
                            diagnostics.push(IdeDiagnostic {
                                module: module_paths[idx].clone(),
                                span: import.span,
                                message: format_type_error(&TypeError::AmbiguousMethod {
                                    receiver: key.0.clone(),
                                    method: key.1.clone(),
                                }),
                                phase: DiagnosticPhase::Type,
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

        // Layer 2: full body type checking (for exact diagnostics) + error-tolerant semantic indexing.
        let view = InterfaceView {
            module_paths: &module_paths,
            exports: &exports,
            signatures: &signatures,
            extra_signatures: None,
        };
        let mut checked_decls = Vec::with_capacity(count);
        let mut all_bodies_clean = true;
        for idx in 0..count {
            let scope = scopes[idx].as_ref().expect("every module has a scope");
            let (decls, body_diags) =
                check_module_bodies(&module_paths[idx], &parsed_modules[idx], scope, &view);
            if !body_diags.is_empty() {
                all_bodies_clean = false;
            }
            for diag in body_diags {
                diagnostics.push(IdeDiagnostic {
                    module: module_paths[idx].clone(),
                    span: diag.span,
                    message: format_type_error(&diag.error),
                    phase: DiagnosticPhase::Type,
                });
            }
            checked_decls.push(decls);
        }

        // Check specialization-time `compile_error` / deferred `where` when bodies are clean.
        if all_bodies_clean {
            let mut mono_roots = Vec::new();
            for (m_idx, mod_decls) in checked_decls.iter().enumerate() {
                for (d_idx, decl) in mod_decls.iter().enumerate() {
                    if decl.type_params.is_empty() {
                        mono_roots.push((
                            format!("{m_idx}:{d_idx}"),
                            InstanceKey {
                                decl: DeclId {
                                    module: ModuleId(m_idx),
                                    index: d_idx,
                                },
                                type_args: Vec::new(),
                            },
                        ));
                    }
                }
            }
            if let Err(spec_errors) = specialize_roots(&checked_decls, &mono_roots) {
                for (module_id, diag) in spec_errors {
                    diagnostics.push(IdeDiagnostic {
                        module: module_paths[module_id.0].clone(),
                        span: diag.span,
                        message: format_type_error(&diag.error),
                        phase: DiagnosticPhase::Type,
                    });
                }
            }
        }

        // Build per-module IDE index (hovers, definitions, local scopes, expr types).
        let mut modules = Vec::with_capacity(count);
        for idx in 0..count {
            let scope = scopes[idx].take().expect("scope present");
            let mut hovers = Vec::new();
            let mut definitions = Vec::new();
            let mut local_scopes = Vec::new();
            let mut expr_types = Vec::new();

            let mut indexer = SemanticIndexer {
                module_path: &module_paths[idx],
                source: &sources[idx],
                scope: &scope,
                view: &view,
                decl_locations: &decl_locations,
                doc_ids: &doc_ids,
                module_indices: &module_indices,
                type_params: Vec::new(),
                type_subst: HashMap::new(),
                locals: HashMap::new(),
                hovers: &mut hovers,
                definitions: &mut definitions,
                local_scopes: &mut local_scopes,
                expr_types: &mut expr_types,
            };
            indexer.index_module(&parsed_modules[idx], &resolved_imports[idx]);

            modules.push(ModuleIndex {
                source: sources[idx].clone(),
                is_document: module_paths[idx].ends_with(".not"),
                symbols: std::mem::take(&mut module_symbols[idx]),
                hovers,
                definitions,
                local_scopes,
                expr_types,
                scope,
            });
        }

        Self {
            diagnostics,
            module_indices,
            module_paths,
            modules,
            exports,
            signatures,
            decl_locations,
        }
    }

    pub fn diagnostics(&self) -> &[IdeDiagnostic] {
        &self.diagnostics
    }

    pub fn diagnostics_for(&self, path: &str) -> Vec<&IdeDiagnostic> {
        let key = normalize_path(path).unwrap_or_else(|| path.to_owned());
        self.diagnostics
            .iter()
            .filter(|d| d.module == key)
            .collect()
    }

    pub fn document_symbols(&self, path: &str) -> &[DocumentSymbol] {
        let Some(idx) = self.find_module(path) else {
            return &[];
        };
        &self.modules[idx].symbols
    }

    pub fn hover(&self, path: &str, offset: u32) -> Option<&HoverInfo> {
        let idx = self.find_module(path)?;
        self.modules[idx]
            .hovers
            .iter()
            .filter(|h| !h.span.is_empty() && h.span.contains(offset))
            .min_by_key(|h| h.span.len())
            .or_else(|| {
                // Also match when cursor sits right at the end of an identifier.
                self.modules[idx]
                    .hovers
                    .iter()
                    .filter(|h| !h.span.is_empty() && h.span.contains_inclusive(offset))
                    .min_by_key(|h| h.span.len())
            })
    }

    pub fn goto_definition(&self, path: &str, offset: u32) -> Option<DefinitionTarget> {
        let idx = self.find_module(path)?;
        let link = self.modules[idx]
            .definitions
            .iter()
            .filter(|d| !d.origin_span.is_empty() && d.origin_span.contains(offset))
            .min_by_key(|d| d.origin_span.len())
            .or_else(|| {
                self.modules[idx]
                    .definitions
                    .iter()
                    .filter(|d| !d.origin_span.is_empty() && d.origin_span.contains_inclusive(offset))
                    .min_by_key(|d| d.origin_span.len())
            })?;
        Some(DefinitionTarget {
            module: link.target_module.clone(),
            span: link.target_span,
        })
    }

    pub fn completions(&self, path: &str, offset: u32) -> Vec<CompletionCandidate> {
        let Some(idx) = self.find_module(path) else {
            return Vec::new();
        };
        let module = &self.modules[idx];
        let offset_usize = (offset as usize).min(module.source.len());

        let Some(ctx) = completion_context(&module.source, offset_usize, module.is_document) else {
            return Vec::new();
        };

        let mut items: Vec<CompletionCandidate> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        if let Some(receiver_expr_text) = ctx.dot_receiver {
            // 1. Try namespace path resolution (e.g. `math.` or `a.b.`)
            if let Some(target_module) = self.resolve_namespace_chain(idx, &receiver_expr_text) {
                let mut entries: Vec<(&String, &Binding)> = self.exports[target_module.0]
                    .iter()
                    .filter(|(name, _)| name.as_str() != DOC_ENTRY)
                    .collect();
                entries.sort_by_key(|(k, _)| *k);
                for (name, binding) in entries {
                    if !name.starts_with(&ctx.partial) || !seen.insert(name.clone()) {
                        continue;
                    }
                    let (kind, detail) = match binding {
                        Binding::Function(decl_id) => (
                            CompletionKind::Function,
                            Some(
                                self.decl_locations[decl_id.module.0][decl_id.index]
                                    .signature
                                    .clone(),
                            ),
                        ),
                        Binding::Namespace(m) => (
                            CompletionKind::Module,
                            Some(format!("module \"{}\"", self.module_paths[m.0])),
                        ),
                    };
                    items.push(CompletionCandidate {
                        label: name.clone(),
                        kind,
                        detail,
                        replace_span: ctx.replace_span,
                    });
                }
                return items;
            }

            // 2. Try receiver type resolution (local variable or inferred expr ending before `.`)
            let receiver_ty = self
                .infer_receiver_type(module, &receiver_expr_text, ctx.dot_offset, offset)
                .or_else(|| {
                    // Fallback: if receiver text is a bare function call `foo()`, lookup its return type.
                    let callee = receiver_expr_text.trim();
                    let name = callee.strip_suffix("()")?;
                    if let Some(Binding::Function(id)) = module.scope.bindings.get(name) {
                        Some(self.signatures[id.module.0][id.index].returns.clone())
                    } else if let Some((_, _, ret)) = lookup_prelude_function(name) {
                        Some(ret)
                    } else {
                        None
                    }
                });

            if let Some(recv_ty) = receiver_ty {
                // User-defined methods on `recv_ty`
                let mut user_methods: Vec<(&String, &MethodSig)> = module
                    .scope
                    .methods
                    .iter()
                    .filter(|((ty, _), _)| *ty == recv_ty)
                    .map(|((_, name), sig)| (name, sig))
                    .collect();
                user_methods.sort_by_key(|(name, _)| *name);
                for (name, sig) in user_methods {
                    if !name.starts_with(&ctx.partial) || !seen.insert(name.clone()) {
                        continue;
                    }
                    items.push(CompletionCandidate {
                        label: name.clone(),
                        kind: CompletionKind::Method,
                        detail: Some(
                            self.decl_locations[sig.id.module.0][sig.id.index]
                                .signature
                                .clone(),
                        ),
                        replace_span: ctx.replace_span,
                    });
                }

                // Built-in methods on `recv_ty`
                for (name, _, params, ret) in builtin_methods_for_type(&recv_ty) {
                    if !name.starts_with(&ctx.partial) || !seen.insert(name.to_owned()) {
                        continue;
                    }
                    let params_str = params
                        .iter()
                        .map(format_type)
                        .collect::<Vec<_>>()
                        .join(", ");
                    items.push(CompletionCandidate {
                        label: name.to_owned(),
                        kind: CompletionKind::Method,
                        detail: Some(format!(
                            "fn {}.{name}(self{}) -> {}",
                            format_type(&recv_ty),
                            if params_str.is_empty() {
                                String::new()
                            } else {
                                format!(", {params_str}")
                            },
                            format_type(&ret)
                        )),
                        replace_span: ctx.replace_span,
                    });
                }
            }
            return items;
        }

        // Bare identifier / `#` completion
        // 1. In-scope local variables & parameters (innermost first)
        for entry in module.local_scopes.iter().rev() {
            if entry.scope_span.contains_inclusive(offset)
                && entry.name.starts_with(&ctx.partial)
                && seen.insert(entry.name.clone())
            {
                items.push(CompletionCandidate {
                    label: entry.name.clone(),
                    kind: CompletionKind::Variable,
                    detail: Some(format_type(&entry.ty)),
                    replace_span: ctx.replace_span,
                });
            }
        }

        // 2. Module scope bindings (local functions, imported functions, namespaces)
        let mut bindings: Vec<(&String, &Binding)> = module
            .scope
            .bindings
            .iter()
            .filter(|(name, _)| name.as_str() != DOC_ENTRY)
            .collect();
        bindings.sort_by_key(|(k, _)| *k);
        for (name, binding) in bindings {
            if !name.starts_with(&ctx.partial) || !seen.insert(name.clone()) {
                continue;
            }
            let (kind, detail) = match binding {
                Binding::Function(decl_id) => (
                    CompletionKind::Function,
                    Some(
                        self.decl_locations[decl_id.module.0][decl_id.index]
                            .signature
                            .clone(),
                    ),
                ),
                Binding::Namespace(m) => (
                    CompletionKind::Module,
                    Some(format!("module \"{}\"", self.module_paths[m.0])),
                ),
            };
            items.push(CompletionCandidate {
                label: name.clone(),
                kind,
                detail,
                replace_span: ctx.replace_span,
            });
        }

        // 3. Prelude built-in functions
        for (name, _, params, ret) in all_prelude_functions() {
            if !name.starts_with(&ctx.partial) || !seen.insert(name.to_owned()) {
                continue;
            }
            let params_str = params
                .iter()
                .map(format_type)
                .collect::<Vec<_>>()
                .join(", ");
            items.push(CompletionCandidate {
                label: name.to_owned(),
                kind: CompletionKind::Function,
                detail: Some(format!("fn {name}({params_str}) -> {}", format_type(&ret))),
                replace_span: ctx.replace_span,
            });
        }

        // 4. Built-in types & keywords in `.notc` code mode
        if !module.is_document && !ctx.after_hash {
            for ty_name in ["Unit", "Bool", "Int", "Float", "String", "Inline", "Block"] {
                if ty_name.starts_with(&ctx.partial) && seen.insert(ty_name.to_owned()) {
                    items.push(CompletionCandidate {
                        label: ty_name.to_owned(),
                        kind: CompletionKind::Type,
                        detail: Some("built-in type".into()),
                        replace_span: ctx.replace_span,
                    });
                }
            }
            for kw in [
                "fn",
                "let",
                "if",
                "else",
                "match",
                "where",
                "import",
                "true",
                "false",
                "compile_error",
            ] {
                if kw.starts_with(&ctx.partial) && seen.insert(kw.to_owned()) {
                    items.push(CompletionCandidate {
                        label: kw.to_owned(),
                        kind: CompletionKind::Keyword,
                        detail: None,
                        replace_span: ctx.replace_span,
                    });
                }
            }
        }

        items
    }

    fn find_module(&self, path: &str) -> Option<usize> {
        let key = normalize_path(path).unwrap_or_else(|| path.to_owned());
        self.module_indices.get(&key).copied()
    }

    fn resolve_namespace_chain(&self, module_idx: usize, chain: &str) -> Option<ModuleId> {
        let mut parts = chain.split('.');
        let first = parts.next()?.trim();
        let &Binding::Namespace(mut current) = self.modules[module_idx].scope.bindings.get(first)?
        else {
            return None;
        };
        for seg in parts {
            let Binding::Namespace(next) = self.exports[current.0].get(seg.trim())? else {
                return None;
            };
            current = *next;
        }
        Some(current)
    }

    fn infer_receiver_type(
        &self,
        module: &ModuleIndex,
        receiver_text: &str,
        dot_offset: u32,
        cursor_offset: u32,
    ) -> Option<Type> {
        // 1. Check expression type ending right at or just before `.`
        if let Some((_, ty)) = module
            .expr_types
            .iter()
            .filter(|(span, _)| span.end <= dot_offset && dot_offset - span.end <= 1)
            .min_by_key(|(span, _)| dot_offset - span.end)
        {
            return Some(ty.clone());
        }
        // 2. Check in-scope local variable matching `receiver_text`
        let trimmed = receiver_text.trim();
        if !trimmed.contains('.') {
            for entry in module.local_scopes.iter().rev() {
                if entry.name == trimmed && entry.scope_span.contains_inclusive(cursor_offset) {
                    return Some(entry.ty.clone());
                }
            }
        }
        None
    }
}

struct CompletionCtx {
    dot_receiver: Option<String>,
    dot_offset: u32,
    partial: String,
    replace_span: Span,
    after_hash: bool,
}

fn completion_context(source: &str, offset: usize, is_document: bool) -> Option<CompletionCtx> {
    let bytes = source.as_bytes();
    let offset = offset.min(bytes.len());

    // Don't complete inside single-line comments in `.notc` files.
    if !is_document {
        let line_start = source[..offset].rfind('\n').map_or(0, |i| i + 1);
        if source[line_start..offset].contains("//") {
            return None;
        }
    }

    // Walk backward across the current identifier fragment.
    let mut ident_start = offset;
    while ident_start > 0 && is_ident_byte(bytes[ident_start - 1]) {
        ident_start -= 1;
    }
    let mut ident_end = offset;
    while ident_end < bytes.len() && is_ident_byte(bytes[ident_end]) {
        ident_end += 1;
    }
    let partial = source[ident_start..offset].to_owned();
    let replace_span = Span::new(ident_start as u32, ident_end as u32);

    // Check for preceding `.` (method or namespace member access).
    let mut before_ident = ident_start;
    while before_ident > 0 && matches!(bytes[before_ident - 1], b' ' | b'\t') {
        before_ident -= 1;
    }
    if before_ident > 0 && bytes[before_ident - 1] == b'.' {
        let dot_pos = before_ident - 1;
        let mut recv_end = dot_pos;
        while recv_end > 0 && matches!(bytes[recv_end - 1], b' ' | b'\t') {
            recv_end -= 1;
        }
        let recv_start = scan_receiver_start(bytes, recv_end);
        let receiver = source[recv_start..recv_end].to_owned();
        if is_document && !preceded_by_hash(bytes, recv_start) {
            return None;
        }
        return Some(CompletionCtx {
            dot_receiver: Some(receiver),
            dot_offset: dot_pos as u32,
            partial,
            replace_span,
            after_hash: preceded_by_hash(bytes, recv_start),
        });
    }

    let after_hash = ident_start > 0 && bytes[ident_start - 1] == b'#';
    if is_document && !after_hash {
        // In `.not` documents, only trigger code completion after `#` (or inside `#(...)`).
        if !inside_hash_parens(&source[..offset]) {
            return None;
        }
    }

    Some(CompletionCtx {
        dot_receiver: None,
        dot_offset: 0,
        partial,
        replace_span,
        after_hash,
    })
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn scan_receiver_start(bytes: &[u8], end: usize) -> usize {
    let mut i = end;
    let mut paren = 0usize;
    let mut bracket = 0usize;
    while i > 0 {
        let b = bytes[i - 1];
        match b {
            b')' => {
                paren += 1;
                i -= 1;
            }
            b'(' if paren > 0 => {
                paren -= 1;
                i -= 1;
            }
            b']' => {
                bracket += 1;
                i -= 1;
            }
            b'[' if bracket > 0 => {
                bracket -= 1;
                i -= 1;
            }
            _ if paren > 0 || bracket > 0 => i -= 1,
            b'.' => i -= 1,
            b if is_ident_byte(b) => i -= 1,
            _ => break,
        }
    }
    i
}

fn preceded_by_hash(bytes: &[u8], pos: usize) -> bool {
    pos > 0 && bytes[pos - 1] == b'#'
}

fn inside_hash_parens(prefix: &str) -> bool {
    let bytes = prefix.as_bytes();
    let mut depth = 0i32;
    let mut i = bytes.len();
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b')' => depth += 1,
            b'(' => {
                if depth == 0 {
                    return i > 0 && bytes[i - 1] == b'#';
                }
                depth -= 1;
            }
            b'\n' => break,
            _ => {}
        }
    }
    false
}

struct SemanticIndexer<'a> {
    module_path: &'a str,
    source: &'a str,
    scope: &'a ModuleScope,
    view: &'a InterfaceView<'a>,
    decl_locations: &'a [Vec<DeclLocation>],
    doc_ids: &'a HashMap<String, HashMap<String, Span>>,
    module_indices: &'a HashMap<String, usize>,
    type_params: Vec<String>,
    type_subst: HashMap<String, Type>,
    locals: HashMap<String, (Type, Span)>,
    hovers: &'a mut Vec<HoverInfo>,
    definitions: &'a mut Vec<DefinitionLink>,
    local_scopes: &'a mut Vec<LocalScopeEntry>,
    expr_types: &'a mut Vec<(Span, Type)>,
}

impl<'a> SemanticIndexer<'a> {
    fn index_module(&mut self, module: &Module, imports: &[(&Import, ModuleId)]) {
        for &(import, target_id) in imports {
            let target_path = &self.view.module_paths[target_id.0];
            self.definitions.push(DefinitionLink {
                origin_span: import.path_span,
                target_module: target_path.clone(),
                target_span: Span::new(0, 0),
            });
            self.hovers.push(HoverInfo {
                span: import.path_span,
                contents: format!("```notc\nmodule \"{target_path}\"\n```"),
            });
            match &import.kind {
                ImportKind::Alias { alias, span } => {
                    self.definitions.push(DefinitionLink {
                        origin_span: *span,
                        target_module: target_path.clone(),
                        target_span: Span::new(0, 0),
                    });
                    self.hovers.push(HoverInfo {
                        span: *span,
                        contents: format!("```notc\nmodule {alias} = \"{target_path}\"\n```"),
                    });
                }
                ImportKind::Members(members) => {
                    let target_exports = &self.view.exports[target_id.0];
                    for member in members {
                        if let Some(&binding) = target_exports.get(&member.name) {
                            self.record_binding_info(member.name_span, binding);
                            if member.alias_span != member.name_span {
                                self.record_binding_info(member.alias_span, binding);
                            }
                        }
                    }
                }
                ImportKind::Default | ImportKind::Wildcard { .. } => {}
            }
        }

        for decl in &module.declarations {
            match decl {
                Decl::Import(_) => {}
                Decl::Function(func) => self.index_function(func),
                Decl::Method(method) => self.index_method(method),
            }
        }
    }

    fn index_function(&mut self, func: &Function) {
        self.locals.clear();
        self.type_subst.clear();
        self.type_params = func.type_params.iter().map(|tp| tp.name.clone()).collect();

        if !func.name_span.is_empty() {
            self.hovers.push(HoverInfo {
                span: func.name_span,
                contents: format!("```notc\n{}\n```", format_function_signature(func)),
            });
        }
        for param in &func.parameters {
            self.locals
                .insert(param.name.clone(), (param.ty.clone(), param.span));
            self.local_scopes.push(LocalScopeEntry {
                scope_span: func.span,
                name: param.name.clone(),
                ty: param.ty.clone(),
            });
            self.hovers.push(HoverInfo {
                span: param.span,
                contents: format!("```notc\n{}: {}\n```", param.name, format_type(&param.ty)),
            });
        }
        if let Some(body) = &func.body {
            self.index_expr(body);
        }
    }

    fn index_method(&mut self, method: &Method) {
        self.locals.clear();
        self.type_subst.clear();
        self.type_params.clear();

        self.hovers.push(HoverInfo {
            span: method.name_span,
            contents: format!("```notc\n{}\n```", format_method_signature(method)),
        });
        self.locals.insert(
            method.receiver_name.clone(),
            (method.receiver_type.clone(), method.receiver_span),
        );
        self.local_scopes.push(LocalScopeEntry {
            scope_span: method.span,
            name: method.receiver_name.clone(),
            ty: method.receiver_type.clone(),
        });
        self.hovers.push(HoverInfo {
            span: method.receiver_span,
            contents: format!(
                "```notc\n{}: {}\n```",
                method.receiver_name,
                format_type(&method.receiver_type)
            ),
        });
        for param in &method.parameters {
            self.locals
                .insert(param.name.clone(), (param.ty.clone(), param.span));
            self.local_scopes.push(LocalScopeEntry {
                scope_span: method.span,
                name: param.name.clone(),
                ty: param.ty.clone(),
            });
            self.hovers.push(HoverInfo {
                span: param.span,
                contents: format!("```notc\n{}: {}\n```", param.name, format_type(&param.ty)),
            });
        }
        self.index_expr(&method.body);
    }

    fn resolve_ty(&self, ty: &Type) -> Type {
        if self.type_subst.is_empty() {
            return ty.clone();
        }
        match ty {
            Type::Param(name) => self
                .type_subst
                .get(name)
                .cloned()
                .unwrap_or_else(|| ty.clone()),
            Type::Function(ps, ret) => Type::Function(
                ps.iter().map(|p| self.resolve_ty(p)).collect(),
                Box::new(self.resolve_ty(ret)),
            ),
            _ => ty.clone(),
        }
    }

    fn resolve_namespace(&self, expr: &Expr) -> Option<ModuleId> {
        match &expr.kind {
            ExprKind::Name(name) => {
                if self.locals.contains_key(name) {
                    return None;
                }
                match self.scope.bindings.get(name) {
                    Some(&Binding::Namespace(id)) => Some(id),
                    _ => None,
                }
            }
            ExprKind::Member(base, member) => {
                let module_id = self.resolve_namespace(base)?;
                match self.view.exports[module_id.0].get(member) {
                    Some(&Binding::Namespace(next)) => Some(next),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn record_binding_info(&mut self, span: Span, binding: Binding) {
        match binding {
            Binding::Function(decl_id) => {
                let loc = &self.decl_locations[decl_id.module.0][decl_id.index];
                self.definitions.push(DefinitionLink {
                    origin_span: span,
                    target_module: loc.module.clone(),
                    target_span: loc.name_span,
                });
                self.hovers.push(HoverInfo {
                    span,
                    contents: format!("```notc\n{}\n```", loc.signature),
                });
            }
            Binding::Namespace(module_id) => {
                let target_path = &self.view.module_paths[module_id.0];
                self.definitions.push(DefinitionLink {
                    origin_span: span,
                    target_module: target_path.clone(),
                    target_span: Span::new(0, 0),
                });
                self.hovers.push(HoverInfo {
                    span,
                    contents: format!("```notc\nmodule \"{target_path}\"\n```"),
                });
            }
        }
    }

    fn index_expr(&mut self, expr: &Expr) -> Option<Type> {
        let inferred = match &expr.kind {
            ExprKind::Unit | ExprKind::EmptyParen => Some(Type::Unit),
            ExprKind::Bool(_) => Some(Type::Bool),
            ExprKind::Int(_) => Some(Type::Int),
            ExprKind::Float(_) => Some(Type::Float),
            ExprKind::String(_) => Some(Type::String),
            ExprKind::CompileError(_) => Some(Type::Never),
            ExprKind::InlineLiteral(pieces) => {
                for p in pieces {
                    self.index_inline_piece(p);
                }
                Some(Type::Inline)
            }
            ExprKind::BlockLiteral(pieces) => {
                for p in pieces {
                    self.index_block_piece(p);
                }
                Some(Type::Block)
            }
            ExprKind::ListLiteral(items) => {
                let mut elem_ty = None;
                for item in items {
                    let ty = self.index_expr(item);
                    if elem_ty.is_none() {
                        elem_ty = ty;
                    }
                }
                elem_ty.map(|t| Type::List(Box::new(t)))
            }
            ExprKind::For {
                binding,
                binding_span,
                iterable,
                body,
            } => {
                let iter_ty = self.index_expr(iterable);
                let elem_ty = match iter_ty {
                    Some(Type::List(inner)) => *inner,
                    _ => Type::Unit,
                };
                let prev = self
                    .locals
                    .insert(binding.clone(), (elem_ty, *binding_span));
                let body_ty = self.index_expr(body);
                if let Some(prev) = prev {
                    self.locals.insert(binding.clone(), prev);
                } else {
                    self.locals.remove(binding);
                }
                body_ty.map(|t| Type::List(Box::new(t)))
            }
            ExprKind::AttachAttrs(_, target) => self.index_expr(target),
            ExprKind::Name(name) => {
                if let Some((ty, def_span)) = self.locals.get(name).cloned() {
                    let resolved = self.resolve_ty(&ty);
                    self.definitions.push(DefinitionLink {
                        origin_span: expr.span,
                        target_module: self.module_path.to_owned(),
                        target_span: def_span,
                    });
                    self.hovers.push(HoverInfo {
                        span: expr.span,
                        contents: format!("```notc\n{name}: {}\n```", format_type(&resolved)),
                    });
                    Some(resolved)
                } else if let Some(&binding) = self.scope.bindings.get(name) {
                    self.record_binding_info(expr.span, binding);
                    match binding {
                        Binding::Function(id) => {
                            let sig = &self.view.signatures[id.module.0][id.index];
                            Some(Type::Function(
                                sig.parameters.clone(),
                                Box::new(sig.returns.clone()),
                            ))
                        }
                        Binding::Namespace(_) => None,
                    }
                } else if let Some((_, params, ret)) = lookup_prelude_function(name) {
                    let params_str = params
                        .iter()
                        .map(format_type)
                        .collect::<Vec<_>>()
                        .join(", ");
                    self.hovers.push(HoverInfo {
                        span: expr.span,
                        contents: format!(
                            "```notc\nfn {name}({params_str}) -> {}\n```",
                            format_type(&ret)
                        ),
                    });
                    Some(Type::Function(params, Box::new(ret)))
                } else {
                    None
                }
            }
            ExprKind::Member(base, member) => {
                self.index_expr(base);
                let member_span = find_member_span(self.source, base.span.end, member)
                    .unwrap_or(expr.span);
                if let Some(module_id) = self.resolve_namespace(base)
                    && let Some(&binding) = self.view.exports[module_id.0].get(member)
                {
                    self.record_binding_info(member_span, binding);
                    match binding {
                        Binding::Function(id) => {
                            let sig = &self.view.signatures[id.module.0][id.index];
                            Some(Type::Function(
                                sig.parameters.clone(),
                                Box::new(sig.returns.clone()),
                            ))
                        }
                        Binding::Namespace(_) => None,
                    }
                } else {
                    None
                }
            }
            ExprKind::Instantiate(target, raw_type_args) => {
                self.index_expr(target);
                let decl_id = match &target.kind {
                    ExprKind::Name(name) if !self.locals.contains_key(name) => {
                        match self.scope.bindings.get(name) {
                            Some(&Binding::Function(id)) => Some(id),
                            _ => None,
                        }
                    }
                    ExprKind::Member(base, member) => self.resolve_namespace(base).and_then(|m| {
                        match self.view.exports[m.0].get(member) {
                            Some(&Binding::Function(id)) => Some(id),
                            _ => None,
                        }
                    }),
                    _ => None,
                };
                if let Some(id) = decl_id {
                    let sig = &self.view.signatures[id.module.0][id.index];
                    if sig.type_params.len() == raw_type_args.len() {
                        let resolved_args: Vec<Type> =
                            raw_type_args.iter().map(|t| self.resolve_ty(t)).collect();
                        let params = sig
                            .parameters
                            .iter()
                            .map(|p| substitute_type(p, &sig.type_params, &resolved_args))
                            .collect();
                        let ret = substitute_type(&sig.returns, &sig.type_params, &resolved_args);
                        Some(Type::Function(params, Box::new(ret)))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            ExprKind::Let(name, init, body) => {
                let init_ty = self.index_expr(init);
                let name_span = find_let_name_span(self.source, expr.span.start, name)
                    .unwrap_or(expr.span);
                let prev = if let Some(ty) = &init_ty {
                    self.hovers.push(HoverInfo {
                        span: name_span,
                        contents: format!("```notc\nlet {name}: {}\n```", format_type(ty)),
                    });
                    self.local_scopes.push(LocalScopeEntry {
                        scope_span: body.span,
                        name: name.clone(),
                        ty: ty.clone(),
                    });
                    self.locals.insert(name.clone(), (ty.clone(), name_span))
                } else {
                    None
                };
                let body_ty = self.index_expr(body);
                if let Some(prev) = prev {
                    self.locals.insert(name.clone(), prev);
                } else if init_ty.is_some() {
                    self.locals.remove(name);
                }
                body_ty
            }
            ExprKind::Seq(first, second) => {
                self.index_expr(first);
                self.index_expr(second)
            }
            ExprKind::Unary(op, operand) => {
                let ty = self.index_expr(operand);
                match op {
                    crate::ast::UnaryOp::Not => Some(Type::Bool),
                    crate::ast::UnaryOp::Neg => ty,
                }
            }
            ExprKind::Binary(op, left, right) => {
                let lt = self.index_expr(left);
                let _rt = self.index_expr(right);
                use crate::ast::BinaryOp as Op;
                match op {
                    Op::Or | Op::And | Op::Eq | Op::Ne | Op::Lt | Op::Le | Op::Gt | Op::Ge => {
                        Some(Type::Bool)
                    }
                    Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Rem => lt,
                }
            }
            ExprKind::If(cond, yes, no) => {
                self.index_expr(cond);
                let yt = self.index_expr(yes);
                let nt = self.index_expr(no);
                yt.or(nt)
            }
            ExprKind::TypeMatch { param, arms, .. } => {
                let mut result = None;
                for arm in arms {
                    match &arm.pattern {
                        TypePattern::Type(pat_ty) => {
                            let resolved = self.resolve_ty(pat_ty);
                            let prev = self.type_subst.insert(param.clone(), resolved);
                            let arm_ty = self.index_expr(&arm.body);
                            if let Some(prev) = prev {
                                self.type_subst.insert(param.clone(), prev);
                            } else {
                                self.type_subst.remove(param);
                            }
                            if result.is_none() && arm_ty != Some(Type::Never) {
                                result = arm_ty;
                            }
                        }
                        TypePattern::Wildcard => {
                            let arm_ty = self.index_expr(&arm.body);
                            if result.is_none() && arm_ty != Some(Type::Never) {
                                result = arm_ty;
                            }
                        }
                    }
                }
                result
            }
            ExprKind::Call(callee, args) => {
                let callee_ty = self.index_expr(callee);
                let arg_tys: Vec<Option<Type>> =
                    args.iter().map(|a| self.index_expr(a)).collect();
                // Generic return inference if calling a generic function directly.
                if let ExprKind::Name(name) = &callee.kind
                    && !self.locals.contains_key(name)
                    && let Some(&Binding::Function(id)) = self.scope.bindings.get(name)
                {
                    let sig = &self.view.signatures[id.module.0][id.index];
                    if !sig.type_params.is_empty() {
                        return self.infer_generic_call_return(sig, &arg_tys);
                    }
                }
                match callee_ty {
                    Some(Type::Function(_, ret)) => Some(*ret),
                    _ => None,
                }
            }
            ExprKind::MethodCall(receiver, method_name, args) => {
                let method_span = find_member_span(self.source, receiver.span.end, method_name)
                    .unwrap_or(expr.span);
                if let Some(module_id) = self.resolve_namespace(receiver) {
                    self.index_expr(receiver);
                    let arg_tys: Vec<Option<Type>> =
                        args.iter().map(|a| self.index_expr(a)).collect();
                    if let Some(&binding) = self.view.exports[module_id.0].get(method_name) {
                        self.record_binding_info(method_span, binding);
                        if let Binding::Function(id) = binding {
                            let sig = &self.view.signatures[id.module.0][id.index];
                            if !sig.type_params.is_empty() {
                                return self.infer_generic_call_return(sig, &arg_tys);
                            }
                            return Some(sig.returns.clone());
                        }
                    }
                    None
                } else {
                    let recv_ty = self.index_expr(receiver);
                    for arg in args {
                        self.index_expr(arg);
                    }
                    let recv_ty = recv_ty?;
                    if let Some(entry) = self
                        .scope
                        .methods
                        .get(&(recv_ty.clone(), method_name.clone()))
                    {
                        let loc = &self.decl_locations[entry.id.module.0][entry.id.index];
                        self.definitions.push(DefinitionLink {
                            origin_span: method_span,
                            target_module: loc.module.clone(),
                            target_span: loc.name_span,
                        });
                        self.hovers.push(HoverInfo {
                            span: method_span,
                            contents: format!("```notc\n{}\n```", loc.signature),
                        });
                        Some(entry.returns.clone())
                    } else if let Some((_, params, ret)) =
                        lookup_builtin_method(&recv_ty, method_name)
                    {
                        let params_str = params
                            .iter()
                            .map(format_type)
                            .collect::<Vec<_>>()
                            .join(", ");
                        self.hovers.push(HoverInfo {
                            span: method_span,
                            contents: format!(
                                "```notc\nfn {}.{method_name}(self{}) -> {}\n```",
                                format_type(&recv_ty),
                                if params_str.is_empty() {
                                    String::new()
                                } else {
                                    format!(", {params_str}")
                                },
                                format_type(&ret)
                            ),
                        });
                        Some(ret)
                    } else {
                        None
                    }
                }
            }
        };

        if let Some(ty) = &inferred {
            self.expr_types.push((expr.span, ty.clone()));
        }
        inferred
    }

    fn infer_generic_call_return(
        &self,
        sig: &FunctionSig,
        arg_tys: &[Option<Type>],
    ) -> Option<Type> {
        let mut inferred: Vec<Option<Type>> = vec![None; sig.type_params.len()];
        for (pat, act) in sig.parameters.iter().zip(arg_tys) {
            if let Some(act) = act
                && let Type::Param(p) = pat
                && let Some(idx) = sig.type_params.iter().position(|tp| tp == p)
            {
                inferred[idx] = Some(act.clone());
            }
        }
        let concrete: Vec<Type> = inferred.into_iter().collect::<Option<_>>()?;
        Some(substitute_type(&sig.returns, &sig.type_params, &concrete))
    }

    fn index_inline_piece(&mut self, piece: &InlinePiece) {
        match piece {
            InlinePiece::Attr { .. }
            | InlinePiece::Text(_, _)
            | InlinePiece::Raw(_, _)
            | InlinePiece::Math(_, _)
            | InlinePiece::Image { .. } => {}
            InlinePiece::Strong(items, _)
            | InlinePiece::Emph(items, _)
            | InlinePiece::Strike(items, _) => {
                for item in items {
                    self.index_inline_piece(item);
                }
            }
            InlinePiece::Link {
                target,
                label,
                span,
            } => {
                for item in label {
                    self.index_inline_piece(item);
                }
                self.index_doc_link(target, *span);
            }
            InlinePiece::Expr(expr, _) => {
                self.index_expr(expr);
            }
            InlinePiece::Attributed { target, .. } => {
                self.index_inline_piece(target);
            }
        }
    }

    fn index_block_piece(&mut self, piece: &BlockPiece) {
        match piece {
            BlockPiece::Paragraph { inlines, .. } => {
                for item in inlines {
                    self.index_inline_piece(item);
                }
            }
            BlockPiece::Heading { title, .. } => {
                for item in title {
                    self.index_inline_piece(item);
                }
            }
            BlockPiece::List { items, .. } => {
                for item in items {
                    for b in item {
                        self.index_block_piece(b);
                    }
                }
            }
            BlockPiece::Quote { body, .. } => {
                for b in body {
                    self.index_block_piece(b);
                }
            }
            BlockPiece::Expr { expr, .. } => {
                self.index_expr(expr);
            }
            BlockPiece::Table { headers, rows, .. } => {
                for cell in headers {
                    for item in cell {
                        self.index_inline_piece(item);
                    }
                }
                for row in rows {
                    for cell in row {
                        for item in cell {
                            self.index_inline_piece(item);
                        }
                    }
                }
            }
            BlockPiece::Attr { .. }
            | BlockPiece::CodeBlock { .. }
            | BlockPiece::Math { .. }
            | BlockPiece::Divider { .. } => {}
        }
    }

    fn index_doc_link(&mut self, target: &str, span: Span) {
        if target.starts_with("http://") || target.starts_with("https://") {
            return;
        }
        let (path_part, anchor) = match target.split_once('#') {
            Some((p, a)) => (p, Some(a)),
            None => (target, None),
        };
        let target_module = if path_part.is_empty() {
            self.module_path.to_owned()
        } else {
            match resolve_import_path(self.module_path, path_part) {
                Ok(resolved) if self.module_indices.contains_key(&resolved) => resolved,
                _ => return,
            }
        };
        let target_span = if let Some(anchor) = anchor.filter(|a| !a.is_empty()) {
            self.doc_ids
                .get(&target_module)
                .and_then(|ids| ids.get(anchor))
                .copied()
                .unwrap_or(Span::new(0, 0))
        } else {
            Span::new(0, 0)
        };
        self.definitions.push(DefinitionLink {
            origin_span: span,
            target_module,
            target_span,
        });
    }
}

fn find_member_span(source: &str, after_offset: u32, member: &str) -> Option<Span> {
    let start_search = (after_offset as usize).min(source.len());
    let rest = &source[start_search..];
    let dot_rel = rest.find('.')?;
    let after_dot = &rest[dot_rel + 1..];
    let ws = after_dot
        .bytes()
        .take_while(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .count();
    if after_dot[ws..].starts_with(member) {
        let start = start_search + dot_rel + 1 + ws;
        let end = start + member.len();
        Some(Span::new(start as u32, end as u32))
    } else {
        None
    }
}

fn find_let_name_span(source: &str, let_start: u32, name: &str) -> Option<Span> {
    let start_search = (let_start as usize).min(source.len());
    let rest = source[start_search..].strip_prefix("let")?;
    let ws = rest
        .bytes()
        .take_while(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .count();
    if rest[ws..].starts_with(name) {
        let start = start_search + 3 + ws;
        let end = start + name.len();
        Some(Span::new(start as u32, end as u32))
    } else {
        None
    }
}

fn format_function_signature(func: &Function) -> String {
    let mut out = format!("fn {}", func.name);
    if !func.type_params.is_empty() {
        out.push('<');
        for (i, tp) in func.type_params.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&tp.name);
            out.push_str(": Type");
        }
        out.push('>');
    }
    out.push('(');
    for (i, p) in func.parameters.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&p.name);
        out.push_str(": ");
        out.push_str(&format_type(&p.ty));
    }
    out.push_str(") -> ");
    out.push_str(&format_type(&func.returns));
    if let Some(where_expr) = &func.where_clause {
        out.push_str(" where ");
        out.push_str(&format_where(where_expr));
    }
    out
}

fn format_method_signature(method: &Method) -> String {
    let mut out = format!(
        "fn {}.{}({}",
        format_type(&method.receiver_type),
        method.name,
        method.receiver_name
    );
    for p in &method.parameters {
        out.push_str(", ");
        out.push_str(&p.name);
        out.push_str(": ");
        out.push_str(&format_type(&p.ty));
    }
    out.push_str(") -> ");
    out.push_str(&format_type(&method.returns));
    out
}

fn format_where(expr: &WhereExpr) -> String {
    match &expr.kind {
        WhereExprKind::Bool(b) => b.to_string(),
        WhereExprKind::TypeEq(a, b) => format!("{} == {}", format_type(a), format_type(b)),
        WhereExprKind::TypeNe(a, b) => format!("{} != {}", format_type(a), format_type(b)),
        WhereExprKind::Not(inner) => format!("!({})", format_where(inner)),
        WhereExprKind::And(a, b) => format!("{} && {}", format_where(a), format_where(b)),
        WhereExprKind::Or(a, b) => format!("{} || {}", format_where(a), format_where(b)),
    }
}

fn extract_document_outline(
    body: &Expr,
    symbols: &mut Vec<DocumentSymbol>,
    ids: &mut HashMap<String, Span>,
) {
    let blocks = match &body.kind {
        ExprKind::BlockLiteral(blocks) => blocks,
        ExprKind::AttachAttrs(_, inner) => match &inner.kind {
            ExprKind::BlockLiteral(blocks) => blocks,
            _ => return,
        },
        _ => return,
    };

    let mut flat_headings: Vec<(u8, DocumentSymbol)> = Vec::new();
    for block in blocks {
        collect_block_outline(block, &mut flat_headings, ids);
    }

    // Nest heading symbols by level.
    let mut stack: Vec<(u8, DocumentSymbol)> = Vec::new();
    let flush_top = |stack: &mut Vec<(u8, DocumentSymbol)>, out: &mut Vec<DocumentSymbol>| {
        let (_, sym) = stack.pop().expect("non-empty heading stack");
        if let Some((_, parent)) = stack.last_mut() {
            parent.children.push(sym);
        } else {
            out.push(sym);
        }
    };
    for (level, sym) in flat_headings {
        while stack.last().is_some_and(|(top_lvl, _)| *top_lvl >= level) {
            flush_top(&mut stack, symbols);
        }
        stack.push((level, sym));
    }
    while !stack.is_empty() {
        flush_top(&mut stack, symbols);
    }
}

fn collect_block_outline(
    block: &BlockPiece,
    headings: &mut Vec<(u8, DocumentSymbol)>,
    ids: &mut HashMap<String, Span>,
) {
    let attrs = match block {
        BlockPiece::Attr { .. } => None,
        BlockPiece::Paragraph { attrs, span, .. }
        | BlockPiece::CodeBlock { attrs, span, .. }
        | BlockPiece::List { attrs, span, .. }
        | BlockPiece::Quote { attrs, span, .. }
        | BlockPiece::Table { attrs, span, .. }
        | BlockPiece::Math { attrs, span, .. }
        | BlockPiece::Divider { attrs, span }
        | BlockPiece::Expr { attrs, span, .. } => {
            record_attr_id(attrs, *span, ids);
            None
        }
        BlockPiece::Heading {
            level,
            title,
            attrs,
            span,
        } => {
            record_attr_id(attrs, *span, ids);
            let title_text = inline_pieces_plain_text(title);
            headings.push((
                *level,
                DocumentSymbol {
                    name: title_text,
                    detail: Some(format!("h{level}")),
                    kind: SymbolKind::Heading,
                    span: *span,
                    selection_span: *span,
                    children: Vec::new(),
                },
            ));
            Some(())
        }
    };
    let _ = attrs;
}

fn record_attr_id(attrs: &[crate::ast::AttrEntry], span: Span, ids: &mut HashMap<String, Span>) {
    for entry in attrs {
        if entry.key == "id"
            && let AttrLiteral::String(id) = &entry.value
        {
            ids.entry(id.clone()).or_insert(span);
        }
    }
}

fn inline_pieces_plain_text(pieces: &[InlinePiece]) -> String {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            InlinePiece::Text(s, _) | InlinePiece::Raw(s, _) | InlinePiece::Math(s, _) => {
                out.push_str(s)
            }
            InlinePiece::Strong(xs, _)
            | InlinePiece::Emph(xs, _)
            | InlinePiece::Strike(xs, _)
            | InlinePiece::Link { label: xs, .. } => {
                out.push_str(&inline_pieces_plain_text(xs));
            }
            InlinePiece::Image { alt, .. } => out.push_str(alt),
            InlinePiece::Attributed { target, .. } => {
                out.push_str(&inline_pieces_plain_text(std::slice::from_ref(target)));
            }
            InlinePiece::Expr(_, _) | InlinePiece::Attr { .. } => {}
        }
    }
    out
}

fn insert_ide_binding(
    bindings: &mut HashMap<String, Binding>,
    name: String,
    binding: Binding,
    span: Span,
    module_path: &str,
    diagnostics: &mut Vec<IdeDiagnostic>,
) {
    match bindings.entry(name) {
        std::collections::hash_map::Entry::Occupied(entry) => {
            diagnostics.push(IdeDiagnostic {
                module: module_path.to_owned(),
                span,
                message: format_type_error(&TypeError::DuplicateBinding(entry.key().clone())),
                phase: DiagnosticPhase::Type,
            });
        }
        std::collections::hash_map::Entry::Vacant(entry) => {
            entry.insert(binding);
        }
    }
}

fn resilient_topological_order(
    module_paths: &[String],
    edges: &[Vec<(&Import, ModuleId)>],
    diagnostics: &mut Vec<IdeDiagnostic>,
) -> Vec<ModuleId> {
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum State {
        Unvisited,
        Visiting,
        Done,
    }

    let mut state = vec![State::Unvisited; module_paths.len()];
    let mut stack: Vec<ModuleId> = Vec::new();
    let mut order = Vec::with_capacity(module_paths.len());

    #[allow(clippy::too_many_arguments)]
    fn visit(
        id: ModuleId,
        module_paths: &[String],
        edges: &[Vec<(&Import, ModuleId)>],
        state: &mut [State],
        stack: &mut Vec<ModuleId>,
        order: &mut Vec<ModuleId>,
        diagnostics: &mut Vec<IdeDiagnostic>,
    ) {
        state[id.0] = State::Visiting;
        stack.push(id);
        for &(import, target) in &edges[id.0] {
            match state[target.0] {
                State::Unvisited => {
                    visit(target, module_paths, edges, state, stack, order, diagnostics);
                }
                State::Visiting => {
                    let start = stack
                        .iter()
                        .position(|&m| m == target)
                        .expect("visiting node is on stack");
                    let mut cycle: Vec<String> = stack[start..]
                        .iter()
                        .map(|m| module_paths[m.0].clone())
                        .collect();
                    cycle.push(module_paths[target.0].clone());
                    diagnostics.push(IdeDiagnostic {
                        module: module_paths[id.0].clone(),
                        span: import.path_span,
                        message: format_type_error(&TypeError::ImportCycle(cycle)),
                        phase: DiagnosticPhase::Type,
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
                diagnostics,
            );
        }
    }
    order
}
