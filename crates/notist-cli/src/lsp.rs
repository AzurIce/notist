use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use notc::ide::{
    CompletionKind, DiagnosticPhase, DocumentSymbol as NotcSymbol, SymbolKind as NotcSymbolKind,
    WorkspaceAnalysis,
};
use notc::{
    BlockPiece, Decl, DocumentFrontend, Expr, ExprKind, InlinePiece, NotistFrontend, Span,
    parse_file_with_frontends, resolve_import_path,
};
use notist_markdown::MarkdownFrontend;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionOptions, CompletionParams, CompletionResponse,
    CompletionTextEdit, Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidChangeWatchedFilesParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, InitializedParams, Location, MarkupContent, MarkupKind,
    OneOf, Position, Range, ServerCapabilities, SymbolKind, TextDocumentSyncCapability,
    TextDocumentSyncKind, TextEdit, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

/// Run the language server over stdio backed by `notc`.
pub async fn serve(_config: Option<PathBuf>) {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        workspace_root: tokio::sync::Mutex::new(None),
        reported: tokio::sync::Mutex::new(HashSet::new()),
        documents: tokio::sync::Mutex::new(HashMap::new()),
        snapshot: tokio::sync::Mutex::new(Arc::new(WorkspaceSnapshot::empty())),
        updates: tokio::sync::Mutex::new(()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}

struct Backend {
    client: Client,
    workspace_root: tokio::sync::Mutex<Option<PathBuf>>,
    reported: tokio::sync::Mutex<HashSet<Url>>,
    documents: tokio::sync::Mutex<HashMap<Url, OpenDocument>>,
    snapshot: tokio::sync::Mutex<Arc<WorkspaceSnapshot>>,
    // Serialize input changes through publication so older batches never
    // overwrite newer snapshots or publish stale diagnostics.
    updates: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
struct OpenDocument {
    source: String,
    version: i32,
}

struct LoadedFile {
    uri: Url,
    source: String,
    lines: LineIndex,
}

struct WorkspaceSnapshot {
    analysis: WorkspaceAnalysis,
    files: HashMap<String, LoadedFile>,
    uri_to_rel: HashMap<Url, String>,
}

impl WorkspaceSnapshot {
    fn empty() -> Self {
        Self {
            analysis: WorkspaceAnalysis::analyze(&[]),
            files: HashMap::new(),
            uri_to_rel: HashMap::new(),
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        #[allow(deprecated)]
        let root = params
            .workspace_folders
            .and_then(|mut folders| folders.drain(..).next())
            .and_then(|folder| folder.uri.to_file_path().ok())
            .or_else(|| params.root_uri.and_then(|uri| uri.to_file_path().ok()));
        *self.workspace_root.lock().await = root;

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

    async fn did_change_watched_files(&self, _: DidChangeWatchedFilesParams) {
        let _update = self.updates.lock().await;
        self.refresh().await;
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let snap = self.current_snapshot().await;
        let Some(rel) = snap.uri_to_rel.get(&uri) else {
            return Ok(None);
        };
        let Some(file) = snap.files.get(rel) else {
            return Ok(None);
        };
        let symbols = snap
            .analysis
            .document_symbols(rel)
            .iter()
            .map(|sym| to_lsp_symbol(sym, &file.source, &file.lines))
            .collect();
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let snap = self.current_snapshot().await;
        let Some(rel) = snap.uri_to_rel.get(uri) else {
            return Ok(None);
        };
        let Some(file) = snap.files.get(rel) else {
            return Ok(None);
        };
        let offset = file.lines.offset(&file.source, position) as u32;
        let Some(hover) = snap.analysis.hover(rel, offset) else {
            return Ok(None);
        };
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: hover.contents.clone(),
            }),
            range: Some(file.lines.range(&file.source, hover.span)),
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let snap = self.current_snapshot().await;
        let Some(rel) = snap.uri_to_rel.get(uri) else {
            return Ok(None);
        };
        let Some(file) = snap.files.get(rel) else {
            return Ok(None);
        };
        let offset = file.lines.offset(&file.source, position) as u32;
        let Some(target) = snap.analysis.goto_definition(rel, offset) else {
            return Ok(None);
        };
        let Some(target_file) = snap.files.get(&target.module) else {
            return Ok(None);
        };
        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: target_file.uri.clone(),
            range: target_file.lines.range(&target_file.source, target.span),
        })))
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let position = params.text_document_position;
        let uri = &position.text_document.uri;
        let path = uri.path();
        if !path.ends_with(".not") && !path.ends_with(".notc") {
            return Ok(None);
        }
        let snap = self.current_snapshot().await;
        let Some(rel) = snap.uri_to_rel.get(uri) else {
            return Ok(None);
        };
        let Some(file) = snap.files.get(rel) else {
            return Ok(None);
        };
        let offset = file.lines.offset(&file.source, position.position) as u32;
        let items = snap
            .analysis
            .completions(rel, offset)
            .into_iter()
            .map(|c| {
                let range = file.lines.range(&file.source, c.replace_span);
                CompletionItem {
                    label: c.label.clone(),
                    kind: Some(match c.kind {
                        CompletionKind::Function => CompletionItemKind::FUNCTION,
                        CompletionKind::Method => CompletionItemKind::METHOD,
                        CompletionKind::Variable => CompletionItemKind::VARIABLE,
                        CompletionKind::Module => CompletionItemKind::MODULE,
                        CompletionKind::Keyword => CompletionItemKind::KEYWORD,
                        CompletionKind::Type => CompletionItemKind::CLASS,
                    }),
                    detail: c.detail,
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit {
                        range,
                        new_text: c.label,
                    })),
                    ..Default::default()
                }
            })
            .collect();
        Ok(Some(CompletionResponse::Array(items)))
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

    async fn current_snapshot(&self) -> Arc<WorkspaceSnapshot> {
        let _update = self.updates.lock().await;
        self.snapshot.lock().await.clone()
    }

    async fn refresh(&self) {
        let documents = self.documents.lock().await.clone();
        let workspace_root = self.workspace_root.lock().await.clone();

        let new_snapshot = build_snapshot(workspace_root.as_deref(), &documents);

        let mut published: HashMap<Url, Vec<Diagnostic>> = documents
            .keys()
            .map(|uri| (uri.clone(), Vec::new()))
            .collect();
        for uri in self.reported.lock().await.iter() {
            published.entry(uri.clone()).or_default();
        }

        for diag in new_snapshot.analysis.diagnostics() {
            if let Some(file) = new_snapshot.files.get(&diag.module) {
                let phase = match diag.phase {
                    DiagnosticPhase::Parse => "parse",
                    DiagnosticPhase::Type => "type",
                };
                published.entry(file.uri.clone()).or_default().push(Diagnostic {
                    range: file.lines.range(&file.source, diag.span),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some(format!("notc:{phase}")),
                    message: diag.message.clone(),
                    ..Default::default()
                });
            }
        }

        *self.reported.lock().await = published
            .iter()
            .filter(|(_, diags)| !diags.is_empty())
            .map(|(uri, _)| uri.clone())
            .collect();
        *self.snapshot.lock().await = Arc::new(new_snapshot);

        for (uri, diagnostics) in published {
            let version = documents.get(&uri).map(|document| document.version);
            self.client
                .publish_diagnostics(uri, diagnostics, version)
                .await;
        }
    }
}

fn build_snapshot(
    workspace_root: Option<&Path>,
    documents: &HashMap<Url, OpenDocument>,
) -> WorkspaceSnapshot {
    let mut fs_paths: Vec<PathBuf> = documents
        .keys()
        .filter_map(|uri| uri.to_file_path().ok())
        .collect();
    if let Some(root) = workspace_root {
        fs_paths.push(root.to_path_buf());
    }
    let root = common_root(&fs_paths);

    let mut files: HashMap<String, LoadedFile> = HashMap::new();
    let mut uri_to_rel: HashMap<Url, String> = HashMap::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut visited: HashSet<String> = HashSet::new();

    // 1. Seed with all open editor documents (unsaved overlays take precedence over disk).
    for (uri, doc) in documents {
        let rel = match uri.to_file_path() {
            Ok(path) => to_virtual_path(&root, &path),
            Err(()) => uri.path().trim_start_matches('/').to_owned(),
        };
        if rel.is_empty() {
            continue;
        }
        uri_to_rel.insert(uri.clone(), rel.clone());
        visited.insert(rel.clone());
        queue.push_back(rel.clone());
        files.insert(
            rel,
            LoadedFile {
                uri: uri.clone(),
                lines: LineIndex::new(&doc.source),
                source: doc.source.clone(),
            },
        );
    }

    // 2. Transitively discover imported `.notc` modules and linked `.not` documents from disk.
    while let Some(rel) = queue.pop_front() {
        let Some(source) = files.get(&rel).map(|f| f.source.clone()) else {
            continue;
        };
        let frontends: [&dyn DocumentFrontend; 2] = [&NotistFrontend, &MarkdownFrontend];
        let parsed = parse_file_with_frontends(&rel, &source, &frontends);
        let mut refs: Vec<String> = Vec::new();
        for decl in &parsed.program.root.declarations {
            match decl {
                Decl::Import(import) => refs.push(import.path.clone()),
                Decl::Function(func) if rel.ends_with(".not") => {
                    if let Some(body) = &func.body {
                        collect_link_paths(body, &mut refs);
                    }
                }
                _ => {}
            }
        }
        for target_raw in refs {
            let Ok(target_rel) = resolve_import_path(&rel, &target_raw) else {
                continue;
            };
            if !visited.insert(target_rel.clone()) {
                continue;
            }
            let target_fs = root.join(&target_rel);
            let Ok(text) = std::fs::read_to_string(&target_fs) else {
                continue;
            };
            let Ok(target_uri) = Url::from_file_path(&target_fs) else {
                continue;
            };
            uri_to_rel.insert(target_uri.clone(), target_rel.clone());
            queue.push_back(target_rel.clone());
            files.insert(
                target_rel,
                LoadedFile {
                    uri: target_uri,
                    lines: LineIndex::new(&text),
                    source: text,
                },
            );
        }
    }

    // 3. Run pure in-memory `notc::ide::WorkspaceAnalysis`.
    let mut pairs: Vec<(&str, &str)> = files
        .iter()
        .map(|(rel, file)| (rel.as_str(), file.source.as_str()))
        .collect();
    pairs.sort_by_key(|(rel, _)| *rel);
    let frontends: [&dyn DocumentFrontend; 2] = [&NotistFrontend, &MarkdownFrontend];
    let analysis = WorkspaceAnalysis::analyze_with_frontends(&pairs, &frontends);

    WorkspaceSnapshot {
        analysis,
        files,
        uri_to_rel,
    }
}

fn collect_link_paths(expr: &Expr, out: &mut Vec<String>) {
    match &expr.kind {
        ExprKind::BlockLiteral(blocks) => {
            for b in blocks {
                collect_block_link_paths(b, out);
            }
        }
        ExprKind::AttachAttrs(_, inner) => collect_link_paths(inner, out),
        _ => {}
    }
}

fn collect_block_link_paths(block: &BlockPiece, out: &mut Vec<String>) {
    match block {
        BlockPiece::Paragraph { inlines, .. } | BlockPiece::Heading { title: inlines, .. } => {
            for inl in inlines {
                collect_inline_link_paths(inl, out);
            }
        }
        BlockPiece::List { items, .. } => {
            for item in items {
                for b in item {
                    collect_block_link_paths(b, out);
                }
            }
        }
        BlockPiece::Quote { body, .. } => {
            for b in body {
                collect_block_link_paths(b, out);
            }
        }
        BlockPiece::Table { headers, rows, .. } => {
            for cell in headers {
                for inl in cell {
                    collect_inline_link_paths(inl, out);
                }
            }
            for row in rows {
                for cell in row {
                    for inl in cell {
                        collect_inline_link_paths(inl, out);
                    }
                }
            }
        }
        BlockPiece::CodeBlock { .. }
        | BlockPiece::Math { .. }
        | BlockPiece::Divider { .. }
        | BlockPiece::Expr { .. } => {}
    }
}

fn collect_inline_link_paths(inline: &InlinePiece, out: &mut Vec<String>) {
    match inline {
        InlinePiece::Strong(xs, _) | InlinePiece::Emph(xs, _) => {
            for x in xs {
                collect_inline_link_paths(x, out);
            }
        }
        InlinePiece::Link { target, label, .. } => {
            if !target.starts_with("http://") && !target.starts_with("https://") {
                let path_part = target.split_once('#').map_or(target.as_str(), |(p, _)| p);
                if !path_part.is_empty() && path_part.ends_with(".not") {
                    out.push(path_part.to_owned());
                }
            }
            for x in label {
                collect_inline_link_paths(x, out);
            }
        }
        InlinePiece::Attributed { target, .. } => collect_inline_link_paths(target, out),
        InlinePiece::Text(_, _)
        | InlinePiece::Raw(_, _)
        | InlinePiece::Math(_, _)
        | InlinePiece::Image { .. }
        | InlinePiece::Expr(_, _) => {}
    }
}

fn common_root(paths: &[PathBuf]) -> PathBuf {
    let dirs: Vec<PathBuf> = paths
        .iter()
        .map(|p| {
            if p.is_dir() {
                p.clone()
            } else {
                p.parent().unwrap_or(p).to_path_buf()
            }
        })
        .collect();
    let Some(first) = dirs.first() else {
        return PathBuf::from("/");
    };
    let mut prefix: Vec<Component<'_>> = first.components().collect();
    for dir in &dirs[1..] {
        let comps: Vec<Component<'_>> = dir.components().collect();
        let common = prefix
            .iter()
            .zip(&comps)
            .take_while(|(a, b)| a == b)
            .count();
        prefix.truncate(common);
    }
    if prefix.is_empty() {
        PathBuf::from("/")
    } else {
        prefix.into_iter().collect()
    }
}

fn to_virtual_path(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let mut parts = Vec::new();
    for comp in rel.components() {
        if let Component::Normal(seg) = comp {
            parts.push(seg.to_string_lossy().into_owned());
        }
    }
    parts.join("/")
}

fn to_lsp_symbol(sym: &NotcSymbol, source: &str, lines: &LineIndex) -> DocumentSymbol {
    #[allow(deprecated)]
    DocumentSymbol {
        name: sym.name.clone(),
        detail: sym.detail.clone(),
        kind: match sym.kind {
            NotcSymbolKind::Function => SymbolKind::FUNCTION,
            NotcSymbolKind::Method => SymbolKind::METHOD,
            NotcSymbolKind::Heading => SymbolKind::STRING,
        },
        tags: None,
        deprecated: None,
        range: lines.range(source, sym.span),
        selection_range: lines.range(source, sym.selection_span),
        children: if sym.children.is_empty() {
            None
        } else {
            Some(
                sym.children
                    .iter()
                    .map(|child| to_lsp_symbol(child, source, lines))
                    .collect(),
            )
        },
    }
}

/// Byte offset ↔ LSP position (line + UTF-16 column).
struct LineIndex {
    starts: Vec<usize>,
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

    fn range(&self, src: &str, span: Span) -> Range {
        Range::new(
            self.position(src, span.start as usize),
            self.position(src, span.end as usize),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_line_index_handles_multibyte_and_surrogate_pairs() {
        let src = "a\n😀中b";
        let lines = LineIndex::new(src);
        // Line 1: 😀 (2 UTF-16 units, 4 UTF-8 bytes), 中 (1 UTF-16 unit, 3 UTF-8 bytes), b (1 unit)
        let pos_zhong = lines.position(src, 2 + 4);
        assert_eq!(pos_zhong, Position::new(1, 2));
        assert_eq!(lines.offset(src, pos_zhong), 6);
        let pos_b = lines.position(src, 2 + 4 + 3);
        assert_eq!(pos_b, Position::new(1, 3));
        assert_eq!(lines.offset(src, pos_b), 9);
    }
}
