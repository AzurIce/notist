use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    CompletionTextEdit, Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentSymbol, DocumentSymbolParams,
    DocumentSymbolResponse, GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents,
    HoverParams, HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams,
    Location, MarkupContent, MarkupKind, OneOf, Position, Range, ServerCapabilities, SymbolKind,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextEdit, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use notist::Resources;
use notist::item::{Ctor, Item, Value};

/// Run the language server over stdio with the CLI configuration.
pub async fn serve(config: Option<std::path::PathBuf>) {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        config: config.clone(),
        reported: tokio::sync::Mutex::new(HashSet::new()),
        documents: tokio::sync::Mutex::new(HashMap::new()),
        snapshots: tokio::sync::Mutex::new(Arc::new(HashMap::new())),
        updates: tokio::sync::Mutex::new(()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}

struct Backend {
    client: Client,
    config: Option<std::path::PathBuf>,
    reported: tokio::sync::Mutex<HashSet<Url>>,
    documents: tokio::sync::Mutex<HashMap<Url, OpenDocument>>,
    snapshots: tokio::sync::Mutex<Arc<HashMap<Url, Arc<DocumentSnapshot>>>>,
    // Serialize input changes through publication: an old batch must never
    // overwrite a newer snapshot or publish stale diagnostics after it.
    updates: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
struct OpenDocument {
    source: String,
    version: i32,
}

struct Declaration {
    name: String,
    span: notist::TextRange,
}

struct DocumentSnapshot {
    source: String,
    lines: LineIndex,
    project: Option<Arc<notist::Environment>>,
    navigation: notist::SourceNavigation,
    analysis: Option<notist::Analysis>,
    index: Option<notist::index::Index>,
    declarations: Vec<Declaration>,
    completion_exclusions: Vec<notist::TextRange>,
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                definition_provider: Some(OneOf::Left(true)),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec!["#".into(), ".".into()]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {}

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let position = params.text_document_position;
        let Some(snapshot) = self.snapshot(&position.text_document.uri).await else {
            return Ok(None);
        };
        let path = position.text_document.uri.path();
        if !path.ends_with(".not") && !path.ends_with(".notc") {
            return Ok(None);
        }
        Ok(Some(CompletionResponse::Array(
            snapshot.complete(position.position, path.ends_with(".notc")),
        )))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.on_change(
            params.text_document.uri,
            params.text_document.text,
            params.text_document.version,
        )
        .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.on_change(
                params.text_document.uri,
                change.text,
                params.text_document.version,
            )
            .await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let _update = self.updates.lock().await;
        self.documents
            .lock()
            .await
            .remove(&params.text_document.uri);
        self.refresh().await;
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn did_change_watched_files(&self, _: tower_lsp::lsp_types::DidChangeWatchedFilesParams) {
        let _update = self.updates.lock().await;
        self.refresh().await;
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let Some(snapshot) = self.snapshot(&uri).await else {
            return Ok(None);
        };
        let src = &snapshot.source;
        let lines = &snapshot.lines;
        if uri.path().ends_with(".notc") {
            let symbols = snapshot
                .declarations
                .iter()
                .map(|function| {
                    #[allow(deprecated)]
                    DocumentSymbol {
                        name: function.name.clone(),
                        detail: None,
                        kind: SymbolKind::FUNCTION,
                        tags: None,
                        deprecated: None,
                        range: lines.range(src, function.span),
                        selection_range: lines.range(src, function.span),
                        children: None,
                    }
                })
                .collect();
            return Ok(Some(DocumentSymbolResponse::Nested(symbols)));
        }
        let Some(analysis) = &snapshot.analysis else {
            return Ok(None);
        };
        let symbols = analysis
            .root()
            .children()
            .iter()
            .filter_map(|item| symbol(item, lines, src))
            .collect();
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some(snapshot) = self.snapshot(uri).await else {
            return Ok(None);
        };
        let src = &snapshot.source;
        let lines = &snapshot.lines;
        let offset = lines.offset(src, position);
        if let Some(project) = &snapshot.project
            && let Some(definition) = snapshot.navigation.definition_at(project.context(), offset)
        {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::PlainText,
                    value: project
                        .modules()
                        .get(&definition.id.module_id())
                        .and_then(|module| {
                            module.source.get(
                                usize::from(definition.span.start())
                                    ..usize::from(definition.span.end()),
                            )
                        })
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            format!(
                                "{}({}) -> {:?}",
                                definition.id,
                                definition
                                    .parameters
                                    .iter()
                                    .map(|parameter| format!(
                                        "{}: {}",
                                        parameter.name, parameter.ty
                                    ))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                definition
                                    .returns
                                    .level()
                                    .unwrap_or(notist::builtins::Level::Inherit)
                            )
                        }),
                }),
                range: None,
            }));
        }
        if uri.path().ends_with(".notc") {
            return Ok(snapshot
                .declarations
                .iter()
                .find(|f| f.span.contains((offset as u32).into()))
                .map(|f| Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::PlainText,
                        value: src[usize::from(f.span.start())..usize::from(f.span.end())]
                            .to_owned(),
                    }),
                    range: Some(lines.range(src, f.span)),
                }));
        }
        let Some(analysis) = &snapshot.analysis else {
            return Ok(None);
        };
        let Some(target) = smallest_at(analysis.root(), offset) else {
            return Ok(None);
        };
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: hover_text(target, analysis.sources()),
            }),
            range: Some(lines.range(src, target.span)),
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let batch = self.batch().await;
        let Some(snapshot) = batch.get(&uri) else {
            return Ok(None);
        };
        let offset = snapshot.lines.offset(&snapshot.source, position);
        if let Some(project) = &snapshot.project
            && let Some(definition) = snapshot.navigation.definition_at(project.context(), offset)
        {
            let Some(module) = project.modules().get(&definition.id.module_id()) else {
                return Ok(None);
            };
            let Ok(uri) = Url::from_file_path(
                project
                    .context()
                    .program()
                    .source_path(&definition.id.module)
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| module.path.clone()),
            ) else {
                return Ok(None);
            };
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri,
                range: LineIndex::new(&module.source).range(&module.source, definition.span),
            })));
        }
        if uri.path().ends_with(".notc") {
            return Ok(None);
        }
        let Some(analysis) = &snapshot.analysis else {
            return Ok(None);
        };
        let Ok(path) = uri.to_file_path() else {
            return Ok(None);
        };
        use notist::vault_index::LinkTarget;
        let (target_path, fragment) =
            match notist::vault_index::link_at(&path, analysis.root(), offset) {
                Some(LinkTarget::SameDoc { item }) => (path, Some(item)),
                Some(LinkTarget::Path { path, item }) => (path, item),
                _ => return Ok(None),
            };
        let Ok(target_uri) = Url::from_file_path(&target_path) else {
            return Ok(None);
        };
        let range = if let Some(fragment) = fragment {
            let Some(target) = batch.get(&target_uri) else {
                return Ok(None);
            };
            let Some(span) = target
                .index
                .as_ref()
                .and_then(|index| index.by_id(&fragment))
            else {
                return Ok(None);
            };
            target.lines.range(&target.source, span)
        } else {
            Range::default()
        };
        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: target_uri,
            range,
        })))
    }
}

impl Backend {
    async fn on_change(&self, uri: Url, source: String, version: i32) {
        let _update = self.updates.lock().await;
        if self
            .documents
            .lock()
            .await
            .get(&uri)
            .is_some_and(|document| document.version >= version)
        {
            return;
        }
        self.documents
            .lock()
            .await
            .insert(uri, OpenDocument { source, version });
        self.refresh().await;
    }

    async fn snapshot(&self, uri: &Url) -> Option<Arc<DocumentSnapshot>> {
        self.batch().await.get(uri).cloned()
    }

    async fn batch(&self) -> Arc<HashMap<Url, Arc<DocumentSnapshot>>> {
        let _update = self.updates.lock().await;
        self.snapshots.lock().await.clone()
    }

    fn vault_from_documents<'a>(
        &self,
        base: &'a notist::FsResources,
        documents: &HashMap<Url, OpenDocument>,
    ) -> notist::Vault<notist::OverlayResources<'a, notist::FsResources>> {
        notist::evict_caches(10);
        let overlays = documents
            .iter()
            .filter_map(|(uri, document)| Some((uri.to_file_path().ok()?, document.source.clone())))
            .collect();
        let mut vault = notist::Vault::new(notist::OverlayResources::new(base, &overlays));
        if let Some(config) = &self.config {
            vault = vault.with_config(config);
        }
        vault
    }

    async fn refresh(&self) {
        let documents = self.documents.lock().await.clone();
        let base = notist::FsResources::default();
        let mut vault = self.vault_from_documents(&base, &documents);
        let mut published: HashMap<Url, Vec<Diagnostic>> = documents
            .keys()
            .map(|uri| (uri.clone(), Vec::new()))
            .collect();
        for uri in self.reported.lock().await.iter() {
            published.entry(uri.clone()).or_default();
        }
        let pin = |error: notist::SourceDiagnostic| {
            let mut diagnostic = error.diagnostic;
            if diagnostic.source.is_none() {
                match notist::Source::new(error.path.to_string_lossy().as_ref(), error.source) {
                    Ok(source) => diagnostic.source = Some(source),
                    Err(error) => {
                        diagnostic.message = error.to_string();
                        diagnostic.span = Default::default();
                    }
                }
            }
            diagnostic
        };
        let mut pending = Vec::new();
        let mut snapshots = HashMap::new();
        for (uri, document) in &documents {
            let path = uri.to_file_path().unwrap_or_else(|_| uri.path().into());
            let mut snapshot = DocumentSnapshot::new(document.source.clone(), &path);
            let diagnostics = match snapshot.analyze(&mut vault, &path) {
                Ok(diagnostics) => diagnostics,
                Err(error) => {
                    for error in source_errors(uri, error) {
                        pending.push((uri.clone(), pin(error)));
                    }
                    if path
                        .extension()
                        .is_some_and(|extension| extension == "notc")
                    {
                        Vec::new()
                    } else {
                        vec![notist::Diagnostic::new(
                            notist::Phase::Semantic,
                            Default::default(),
                            "source imports or output configuration are invalid",
                        )]
                    }
                }
            };
            snapshots.insert(uri.clone(), Arc::new(snapshot));
            pending.extend(diagnostics.into_iter().map(|d| (uri.clone(), d)));
        }
        // Resolve direct cross-file anchors from this same input batch, including
        // unopened files. Queries must not perform IO or execute extensions.
        // Do not recursively analyze a whole linked workspace on each keystroke.
        let anchor_targets: HashSet<_> = snapshots
            .iter()
            .flat_map(|(uri, snapshot)| {
                let path = uri.to_file_path().unwrap_or_else(|_| uri.path().into());
                snapshot.analysis.iter().flat_map(move |analysis| {
                    let path = path.clone();
                    analysis.root().descendants().filter_map(move |node| {
                        match notist::vault_index::link_target(&path, node) {
                            Some(notist::vault_index::LinkTarget::Path {
                                path,
                                item: Some(_),
                            }) => Url::from_file_path(path).ok(),
                            _ => None,
                        }
                    })
                })
            })
            .collect();
        for uri in anchor_targets {
            if snapshots.contains_key(&uri) {
                continue;
            }
            let Ok(path) = uri.to_file_path() else {
                continue;
            };
            if !path.starts_with(vault.resources().root()) || !vault.supports(&path) {
                continue;
            }
            let Ok(source) = vault.source(&path) else {
                continue;
            };
            let mut snapshot = DocumentSnapshot::new(source, &path);
            if snapshot.analyze(&mut vault, &path).is_ok() {
                snapshots.insert(uri, Arc::new(snapshot));
            }
        }
        let mut line_indexes = LineIndexes::default();
        // Borrow pending diagnostics until every conversion finishes. Their
        // pinned revisions (and the open-document snapshot) outlive the indexes.
        for (fallback, diagnostic) in &mut pending {
            let uri = diagnostic
                .source
                .as_deref()
                .and_then(|path| Url::from_file_path(path).ok())
                .unwrap_or(fallback.clone());
            let source = match diagnostic.source.as_ref() {
                Some(source) => Some(match source.text() {
                    Some(text) => text.as_ref(),
                    None => {
                        diagnostic.message.push_str(" (source text is unavailable)");
                        diagnostic.span = Default::default();
                        ""
                    }
                }),
                None => documents
                    .get(&uri)
                    .or_else(|| documents.get(&*fallback))
                    .map(|document| document.source.as_str()),
            };
            if let Some(source) = source {
                published.entry(uri).or_default().push(lsp_diagnostic(
                    source,
                    diagnostic,
                    line_indexes.get(source),
                ));
            }
        }
        for diagnostics in published.values_mut() {
            let mut unique = Vec::new();
            for diagnostic in diagnostics.drain(..) {
                if !unique.contains(&diagnostic) {
                    unique.push(diagnostic);
                }
            }
            *diagnostics = unique;
        }
        *self.reported.lock().await = published
            .iter()
            .filter(|(_, diagnostics)| !diagnostics.is_empty())
            .map(|(uri, _)| uri.clone())
            .collect();
        *self.snapshots.lock().await = Arc::new(snapshots);
        for (uri, diagnostics) in published {
            let version = documents.get(&uri).map(|document| document.version);
            self.client
                .publish_diagnostics(uri, diagnostics, version)
                .await;
        }
    }
}

impl DocumentSnapshot {
    fn complete(&self, position: Position, code: bool) -> Vec<CompletionItem> {
        let offset = self.lines.offset(&self.source, position);
        if self
            .completion_exclusions
            .iter()
            .any(|span| usize::from(span.start()) < offset && offset <= usize::from(span.end()))
        {
            return Vec::new();
        }
        let Some((start, end)) = completion_path(&self.source, offset, code) else {
            return Vec::new();
        };
        let prefix = &self.source[start..offset];
        let (namespace, partial) = prefix.rsplit_once('.').unwrap_or(("", prefix));
        let fallback = notist::Context::default();
        let context = self
            .project
            .as_ref()
            .map(|p| p.context())
            .unwrap_or(&fallback);
        let mut module = context.root().map(str::to_owned);
        if !namespace.is_empty() {
            for (index, part) in namespace.split('.').enumerate() {
                if index == 0 && part == "notist" {
                    module = Some("notist".into());
                    continue;
                }
                if index == 0 && part == "self" {
                    continue;
                }
                let Some(notist::ImportBinding::Module(id)) = module
                    .as_deref()
                    .and_then(|name| context.public_bindings(name).get(part).cloned())
                else {
                    return Vec::new();
                };
                module = context
                    .identities()
                    .module(id)
                    .ok()
                    .map(|m| m.name().to_owned());
            }
        }
        let mut bindings = if namespace.is_empty() {
            context.public_bindings("notist")
        } else {
            Default::default()
        };
        if let Some(module) = module {
            bindings.extend(context.public_bindings(&module));
        }
        let edit_start = offset - partial.len();
        let range = self.lines.range(
            &self.source,
            notist::TextRange::new((edit_start as u32).into(), (end as u32).into()),
        );
        let mut items: Vec<_> = bindings
            .into_iter()
            .filter_map(|(name, binding)| {
                if !name.starts_with(partial) {
                    return None;
                }
                let (kind, detail) = match binding {
                    notist::ImportBinding::Module(_) => (CompletionItemKind::MODULE, None),
                    notist::ImportBinding::Function(id) => {
                        let definition = context.registry().definition(id)?;
                        let detail = format!(
                            "{}({}) -> {}",
                            name,
                            definition
                                .parameters
                                .iter()
                                .map(|p| format!("{}: {}", p.name, p.ty))
                                .collect::<Vec<_>>()
                                .join(", "),
                            definition.returns
                        );
                        (CompletionItemKind::FUNCTION, Some(detail))
                    }
                };
                Some(CompletionItem {
                    label: name.clone(),
                    kind: Some(kind),
                    detail,
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range,
                        new_text: name,
                    })),
                    ..Default::default()
                })
            })
            .collect();
        if namespace.is_empty() {
            for name in ["notist", "self"] {
                if name.starts_with(partial) && (name != "self" || context.root().is_some()) {
                    items.push(CompletionItem {
                        label: name.into(),
                        kind: Some(CompletionItemKind::MODULE),
                        text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                            range,
                            new_text: name.into(),
                        })),
                        ..Default::default()
                    });
                }
            }
        }
        items.sort_by(|a, b| a.label.cmp(&b.label));
        items
    }

    fn new(source: String, path: &std::path::Path) -> Self {
        Self {
            completion_exclusions: completion_exclusions(
                &source,
                path.extension().is_some_and(|e| e == "notc"),
            ),
            lines: LineIndex::new(&source),
            source,
            project: None,
            navigation: Default::default(),
            analysis: None,
            index: None,
            declarations: Vec::new(),
        }
    }

    fn analyze<R: notist::Resources>(
        &mut self,
        vault: &mut notist::Vault<R>,
        path: &std::path::Path,
    ) -> std::result::Result<Vec<notist::Diagnostic>, notist::VaultError> {
        let diagnostics = if path
            .extension()
            .is_some_and(|extension| extension == "notc")
        {
            let analysis = vault.analyze_code(path, &self.source)?;
            self.project = analysis.environment().cloned();
            self.navigation = analysis.navigation();
            if let Some(definitions) = &analysis.definitions {
                self.declarations = definitions
                    .functions
                    .iter()
                    .map(|function| Declaration {
                        name: function.id.name.clone(),
                        span: function.span,
                    })
                    .collect();
            } else {
                // Keep syntax-only outlines usable while imports/signatures are
                // broken. Parse once per input batch, never once per query.
                let parse = notist::syntax::code::parse_module(
                    &self.source,
                    &notist::syntax::markup::MarkupContentParser,
                );
                if let Some(module) = notist::syntax::code::ast::Module::cast(parse.syntax()) {
                    self.declarations = module
                        .functions()
                        .filter_map(|function| {
                            Some(Declaration {
                                name: function.name()?.text().into(),
                                span: function.range(),
                            })
                        })
                        .collect();
                }
            }
            analysis.diagnostics
        } else if path.file_name().is_some_and(|name| name == "Notist.toml") {
            notist::environment::parse_config(&self.source)
                .err()
                .unwrap_or_default()
        } else {
            let tooling = vault.analyze_for_tooling(path, &self.source)?;
            self.index = Some(tooling.index);
            self.project = Some(tooling.environment);
            self.navigation = tooling.navigation;
            let diagnostics = tooling.analysis.diagnostics().to_vec();
            self.analysis = Some(tooling.analysis);
            diagnostics
        };
        Ok(diagnostics)
    }
}

/// Cache syntax-only exclusion ranges; editor queries never parse or evaluate.
fn completion_exclusions(source: &str, code: bool) -> Vec<notist::TextRange> {
    use notist::syntax::tree::SyntaxKind as K;
    let parse = if code {
        notist::syntax::code::parse_module(source, &notist::syntax::markup::MarkupContentParser)
    } else {
        notist::syntax::markup::parse_document(source)
    };
    parse
        .syntax()
        .descendants_with_tokens()
        .filter(|element| {
            matches!(
                element.kind(),
                K::Str
                    | K::PathLiteral
                    | K::LineComment
                    | K::BlockComment
                    | K::Raw
                    | K::RawInline
                    | K::Math
            )
        })
        .map(|element| element.text_range())
        .collect()
}

/// Preserve the namespace and replace the entire final identifier, even when
/// completion is requested mid-word.
fn completion_path(source: &str, offset: usize, code: bool) -> Option<(usize, usize)> {
    let is_name = |c: char| c.is_alphanumeric() || c == '_';
    let before = source.get(..offset)?;
    let start = before
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_name(*c) || *c == '.')
        .last()
        .map_or(offset, |(i, _)| i);
    if !code
        && (start == 0 || !source[..start].ends_with('#') || source[..start - 1].ends_with('\\'))
    {
        return None;
    }
    let end = source[offset..]
        .char_indices()
        .take_while(|(_, c)| is_name(*c))
        .last()
        .map_or(offset, |(i, c)| offset + i + c.len_utf8());
    Some((start, end))
}

fn source_errors(uri: &Url, error: notist::VaultError) -> Vec<notist::SourceDiagnostic> {
    match error {
        notist::VaultError::Environment(errors) => errors,
        error => vec![notist::SourceDiagnostic {
            path: uri.to_file_path().unwrap_or_else(|_| uri.path().into()),
            source: String::new(),
            diagnostic: notist::Diagnostic::new(
                notist::Phase::Semantic,
                Default::default(),
                error.to_string(),
            ),
        }],
    }
}

/// The smallest item whose span contains `offset`.
fn smallest_at(item: &Item, offset: usize) -> Option<&Item> {
    for child in item.children() {
        if child.span.contains(notist::TextSize::from(offset as u32)) {
            return Some(smallest_at(child, offset).unwrap_or(child));
        }
    }
    None
}

fn hover_text(item: &Item, sources: &notist::Sources) -> String {
    let mut out = format!(
        "`{}` ({:?})",
        item.ctor()
            .function_id()
            .map(|id| sources
                .definitions()
                .function(id)
                .map(ToString::to_string)
                .unwrap_or_else(|error| error.to_string()))
            .unwrap_or_else(|| item
                .ctor()
                .name(sources.definitions())
                .map(|name| name.into_owned())
                .unwrap_or_else(|error| error.to_string())),
        item.level
    );
    for (k, v) in item.fields().iter() {
        out.push_str(&format!("\n- `{k}` = {v}"));
    }
    for (k, v) in item.attrs.iter() {
        out.push_str(&format!("\n- `@{k}` = {v}"));
    }
    out
}

/// A heading → a symbol; sections nest. Section names are their heading text.
fn symbol(item: &Item, lines: &LineIndex, src: &str) -> Option<DocumentSymbol> {
    let (name, range, selection, children) = match item.ctor() {
        Ctor::Section => {
            let heading = item.children().first()?;
            let name = text_of(heading);
            (
                name,
                lines.range(src, item.span),
                lines.range(src, heading.span),
                item.children()[1..]
                    .iter()
                    .filter_map(|c| symbol(c, lines, src))
                    .collect(),
            )
        }
        Ctor::Heading => (
            text_of(item),
            lines.range(src, item.span),
            lines.range(src, item.span),
            Vec::new(),
        ),
        _ => return None,
    };
    #[allow(deprecated)]
    Some(DocumentSymbol {
        name,
        detail: None,
        kind: SymbolKind::NAMESPACE,
        tags: None,
        deprecated: None,
        range,
        selection_range: selection,
        children: Some(children),
    })
}

/// The text content of a heading: its children's text fields, joined.
fn text_of(item: &Item) -> String {
    item.children()
        .iter()
        .filter_map(|c| match c.field("text") {
            Some(Value::Str(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Byte offset ↔ LSP position (line + UTF-16 column).
struct LineIndex {
    starts: Vec<usize>,
}

/// One diagnostic publication's indexes, keyed by immutable text storage rather
/// than file name or SourceId (both can have multiple retained revisions).
#[derive(Default)]
struct LineIndexes<'a> {
    entries: HashMap<(usize, usize), (&'a str, LineIndex)>,
}
impl<'a> LineIndexes<'a> {
    fn get(&mut self, source: &'a str) -> &LineIndex {
        &self
            .entries
            .entry((source.as_ptr() as usize, source.len()))
            .or_insert_with(|| (source, LineIndex::new(source)))
            .1
    }
}

impl LineIndex {
    fn new(src: &str) -> Self {
        let mut starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Self { starts }
    }

    fn position(&self, src: &str, offset: usize) -> Position {
        let mut offset = offset.min(src.len());
        while !src.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let col = src[self.starts[line]..offset]
            .chars()
            .map(|c| c.len_utf16())
            .sum::<usize>();
        Position::new(line as u32, col as u32)
    }

    fn offset(&self, src: &str, position: Position) -> usize {
        let line = (position.line as usize).min(self.starts.len() - 1);
        let start = self.starts[line];
        let end = self.starts.get(line + 1).copied().unwrap_or(src.len());
        let mut col = 0usize;
        for (i, c) in src[start..end].char_indices() {
            if col >= position.character as usize {
                return start + i;
            }
            col += c.len_utf16();
        }
        end
    }

    fn range(&self, src: &str, span: notist::TextRange) -> Range {
        Range::new(
            self.position(src, usize::from(span.start())),
            self.position(src, usize::from(span.end())),
        )
    }
}

fn lsp_diagnostic(source: &str, diagnostic: &notist::Diagnostic, lines: &LineIndex) -> Diagnostic {
    Diagnostic {
        range: lines.range(source, diagnostic.span),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some(format!("notist:{}", diagnostic.phase)),
        message: diagnostic.message.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_indexes_share_text_storage_but_never_confuse_revisions() {
        let old = notist::Source::new("same.not", "a\n😀中").unwrap();
        let new = notist::Source::new("same.not", "😀\n中a").unwrap();
        assert_eq!(old.id(), new.id());
        let clone = old.clone();
        let mut indexes = LineIndexes::default();
        for source in [&old, &clone, &new] {
            let text = source.text().unwrap();
            let cached = indexes.get(text);
            for offset in 0..=text.len() {
                assert_eq!(
                    cached.position(text, offset),
                    LineIndex::new(text).position(text, offset)
                );
            }
        }
        assert_eq!(indexes.entries.len(), 2);
    }

    #[test]
    fn smallest_at_finds_innermost() {
        let (item, _, _sources) = notist::Pipeline::default()
            .analyze(
                "test.not",
                "内容 *粗体* 文字\n",
                &notist::Context::default(),
            )
            .unwrap()
            .into_parts();
        // 体 at byte 11，粗 at 8
        let found = smallest_at(&item, 11).unwrap();
        assert_eq!(found.ctor().name(&Default::default()).unwrap(), "Text");
        assert_eq!(found.field("text").unwrap().to_string(), "\"粗体\"");
        let found = smallest_at(&item, 8).unwrap();
        assert_eq!(found.ctor().name(&Default::default()).unwrap(), "Text");
        // byte 2 (容) → 内容
        let found = smallest_at(&item, 2).unwrap();
        assert_eq!(found.field("text").unwrap().to_string(), "\"内容 \"");
    }
}
