use std::collections::{HashMap, HashSet};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, DocumentSymbol, DocumentSymbolParams, DocumentSymbolResponse,
    GotoDefinitionParams, GotoDefinitionResponse, Hover, HoverContents, HoverParams,
    HoverProviderCapability, InitializeParams, InitializeResult, InitializedParams, Location,
    MarkupContent, MarkupKind, OneOf, Position, Range, ServerCapabilities, SymbolKind,
    TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

use notist::item::{Ctor, Item, Value};

/// Run the language server over stdio with the CLI configuration.
pub async fn serve_with_config(config: Option<std::path::PathBuf>) {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        config: config.clone(),
        reported: tokio::sync::Mutex::new(HashSet::new()),
        documents: tokio::sync::Mutex::new(HashMap::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}

struct Backend {
    client: Client,
    config: Option<std::path::PathBuf>,
    reported: tokio::sync::Mutex<HashSet<Url>>,
    documents: tokio::sync::Mutex<HashMap<Url, String>>,
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
        self.on_change(params.text_document.uri, params.text_document.text)
            .await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Some(change) = params.content_changes.into_iter().last() {
            self.on_change(params.text_document.uri, change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .lock()
            .await
            .remove(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
        self.refresh().await;
    }

    async fn did_change_watched_files(&self, _: tower_lsp::lsp_types::DidChangeWatchedFilesParams) {
        self.refresh().await;
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        if uri.path().ends_with(".notc") {
            let documents = self.documents.lock().await;
            let Some(src) = documents.get(&uri) else {
                return Ok(None);
            };
            let lines = LineIndex::new(src);
            let parse = notist::syntax::parse_module(src);
            let module = notist::syntax::ast::Module::cast(parse.syntax()).unwrap();
            let symbols = module
                .functions()
                .filter_map(|function| {
                    #[allow(deprecated)]
                    Some(DocumentSymbol {
                        name: function.name()?.text().into(),
                        detail: None,
                        kind: SymbolKind::FUNCTION,
                        tags: None,
                        deprecated: None,
                        range: lines.range(src, function.range()),
                        selection_range: lines.range(src, function.range()),
                        children: None,
                    })
                })
                .collect();
            return Ok(Some(DocumentSymbolResponse::Nested(symbols)));
        }
        let Some((src, item)) = self.analyzed(&uri).await else {
            return Ok(None);
        };
        let lines = LineIndex::new(&src);
        let symbols = item
            .children
            .iter()
            .filter_map(|item| symbol(item, &lines, &src))
            .collect();
        Ok(Some(DocumentSymbolResponse::Nested(symbols)))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        if uri.path().ends_with(".notc") {
            let documents = self.documents.lock().await;
            let Some(src) = documents.get(uri) else {
                return Ok(None);
            };
            let lines = LineIndex::new(src);
            let offset = lines.offset(src, position);
            let parse = notist::syntax::parse_module(src);
            let module = notist::syntax::ast::Module::cast(parse.syntax()).unwrap();
            let target = module
                .functions()
                .find(|function| function.range().contains((offset as u32).into()));
            return Ok(target.map(|function| Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::PlainText,
                    value: function.syntax().text().to_string(),
                }),
                range: Some(lines.range(src, function.range())),
            }));
        }
        let Some((src, item)) = self.analyzed(uri).await else {
            return Ok(None);
        };
        let lines = LineIndex::new(&src);
        let offset = lines.offset(&src, position);
        if let Some(name) = call_path_at(uri, &src, offset)
            && let Ok(project) = self.project(uri).await
            && let Ok(definition) = project.registry().resolve(&name)
        {
            return Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::PlainText,
                    value: project
                        .packages()
                        .get(&definition.id.package)
                        .and_then(|package| {
                            package.source.get(
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
                                definition.returns.base_level()
                            )
                        }),
                }),
                range: None,
            }));
        }
        let Some(target) = smallest_at(&item, offset) else {
            return Ok(None);
        };
        Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: hover_text(target),
            }),
            range: Some(lines.range(&src, target.span)),
        }))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let Some((src, item)) = self.analyzed(&uri).await else {
            return Ok(None);
        };
        let lines = LineIndex::new(&src);
        let offset = lines.offset(&src, position);
        if let Some(name) = call_path_at(&uri, &src, offset) {
            let Some(project) = self.project(&uri).await.ok() else {
                return Ok(None);
            };
            let Ok(definition) = project.registry().resolve(&name) else {
                return Ok(None);
            };
            let Some(package) = project.packages().get(&definition.id.package) else {
                return Ok(None);
            };
            let Ok(uri) = Url::from_file_path(package.root.join("lib.notc")) else {
                return Ok(None);
            };
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri,
                range: LineIndex::new(&package.source).range(&package.source, definition.span),
            })));
        }
        let Some(target) = smallest_at(&item, offset) else {
            return Ok(None);
        };
        if target.ctor != Ctor::Link {
            return Ok(None);
        }
        let Some(Value::Str(target_text)) = target.fields.get("target") else {
            return Ok(None);
        };
        // `#item`：当前文档内的 id
        if let Some(item_id) = target_text.strip_prefix('#') {
            let index = notist::index::Index::build(&item, &mut Vec::new());
            let Some(span) = index.by_id(item_id) else {
                return Ok(None);
            };
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri,
                range: lines.range(&src, span),
            })));
        }
        // 路径链接：跳到目标文件开头
        if target_text.contains("://") {
            return Ok(None);
        }
        let path_part = target_text.split('#').next().unwrap_or("");
        let doc_dir = uri
            .to_file_path()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()));
        let Some(dir) = doc_dir else { return Ok(None) };
        let target_path = notist::resources::normalize(&dir.join(path_part));
        let Ok(target_uri) = Url::from_file_path(&target_path) else {
            return Ok(None);
        };
        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: target_uri,
            range: Range::default(),
        })))
    }
}

impl Backend {
    async fn on_change(&self, uri: Url, text: String) {
        self.documents
            .lock()
            .await
            .insert(uri.clone(), text.clone());
        self.refresh().await;
    }

    async fn project(
        &self,
        uri: &Url,
    ) -> std::result::Result<notist::Environment, Vec<notist::SourceDiagnostic>> {
        self.with_vault(|vault| {
            vault
                .environment_for(uri.to_file_path().unwrap_or_else(|_| "document.not".into()))
                .cloned()
        })
        .await
        .map_err(|error| source_errors(uri, error))
    }

    async fn with_vault<T>(
        &self,
        operation: impl FnOnce(
            &mut notist::Vault<notist::OverlayResources<'_, notist::FsResources>>,
        ) -> std::result::Result<T, notist::VaultError>,
    ) -> std::result::Result<T, notist::VaultError> {
        let documents = self.documents.lock().await.clone();
        let base = notist::FsResources::default();
        let mut vault = self.vault_from_documents(&base, &documents);
        operation(&mut vault)
    }

    fn vault_from_documents<'a>(
        &self,
        base: &'a notist::FsResources,
        documents: &HashMap<Url, String>,
    ) -> notist::Vault<notist::OverlayResources<'a, notist::FsResources>> {
        let overlays = documents
            .iter()
            .filter_map(|(uri, source)| Some((uri.to_file_path().ok()?, source.clone())))
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
        for (uri, source) in &documents {
            let diagnostics = if uri.path().ends_with(".notc") {
                notist::analyze_module("package", source)
                    .err()
                    .unwrap_or_default()
            } else if uri.path().ends_with("Notist.toml") {
                notist::environment::parse_config(source)
                    .err()
                    .unwrap_or_default()
            } else {
                match vault
                    .analyze(
                        uri.to_file_path().unwrap_or_else(|_| uri.path().into()),
                        source,
                    )
                    .map_err(|error| source_errors(uri, error))
                {
                    Ok(analysis) => analysis.into_parts().1,
                    Err(errors) => {
                        for error in errors {
                            if let Ok(error_uri) = Url::from_file_path(&error.path) {
                                published
                                    .entry(error_uri)
                                    .or_default()
                                    .push(lsp_diagnostic(&error.source, &error.diagnostic));
                            }
                        }
                        vec![notist::Diagnostic::new(
                            notist::Phase::Semantic,
                            Default::default(),
                            "project configuration or package declarations are invalid",
                        )]
                    }
                }
            };
            published.entry(uri.clone()).or_default().extend(
                diagnostics
                    .iter()
                    .map(|diagnostic| lsp_diagnostic(source, diagnostic)),
            );
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
        for (uri, diagnostics) in published {
            self.client
                .publish_diagnostics(uri, diagnostics, None)
                .await;
        }
    }

    async fn analyzed(&self, uri: &Url) -> Option<(String, Item)> {
        let documents = self.documents.lock().await.clone();
        let src = documents.get(uri)?.clone();
        let base = notist::FsResources::default();
        let mut vault = self.vault_from_documents(&base, &documents);
        let (item, _) = vault
            .analyze(
                uri.to_file_path().unwrap_or_else(|_| uri.path().into()),
                &src,
            )
            .ok()?
            .into_parts();
        Some((src, item))
    }
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
    for child in &item.children {
        if child.span.contains(notist::TextSize::from(offset as u32)) {
            return Some(smallest_at(child, offset).unwrap_or(child));
        }
    }
    None
}

fn hover_text(item: &Item) -> String {
    let mut out = format!(
        "`{}` ({:?})",
        item.ctor
            .function_id()
            .map(|id| id.to_string())
            .unwrap_or_else(|| item.ctor.name().into_owned()),
        item.level
    );
    for (k, v) in item.fields.iter() {
        out.push_str(&format!("\n- `{k}` = {v}"));
    }
    for (k, v) in item.attrs.iter() {
        out.push_str(&format!("\n- `@{k}` = {v}"));
    }
    out
}

/// A heading → a symbol; sections nest. Section names are their heading text.
fn symbol(item: &Item, lines: &LineIndex, src: &str) -> Option<DocumentSymbol> {
    let (name, range, selection, children) = match item.ctor {
        Ctor::Section => {
            let heading = item.children.first()?;
            let name = text_of(heading);
            (
                name,
                lines.range(src, item.span),
                lines.range(src, heading.span),
                item.children[1..]
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
    item.children
        .iter()
        .filter_map(|c| match c.fields.get("text") {
            Some(Value::Str(text)) => Some(text.as_str()),
            _ => None,
        })
        .collect()
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
        let offset = offset.min(src.len());
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

fn call_path_at(uri: &Url, source: &str, offset: usize) -> Option<String> {
    if !uri.path().ends_with(".not") {
        return None;
    }
    notist::syntax::parse_document(source)
        .syntax()
        .descendants()
        .filter_map(notist::syntax::ast::CodeCall::cast)
        .find_map(|call| {
            let path = call.path()?;
            path.range()
                .contains((offset as u32).into())
                .then(|| path.text())
        })
}

fn lsp_diagnostic(source: &str, diagnostic: &notist::Diagnostic) -> Diagnostic {
    Diagnostic {
        range: LineIndex::new(source).range(source, diagnostic.span),
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
    fn smallest_at_finds_innermost() {
        let (item, _) = notist::Pipeline::default()
            .analyze(
                "test.not",
                "内容 *粗体* 文字\n",
                notist::builtins::registry(),
            )
            .unwrap()
            .into_parts();
        // 体 at byte 11，粗 at 8
        let found = smallest_at(&item, 11).unwrap();
        assert_eq!(found.ctor.name(), "Text");
        assert_eq!(found.fields.get("text").unwrap().to_string(), "\"粗体\"");
        let found = smallest_at(&item, 8).unwrap();
        assert_eq!(found.ctor.name(), "Text");
        // byte 2 (容) → 内容
        let found = smallest_at(&item, 2).unwrap();
        assert_eq!(found.fields.get("text").unwrap().to_string(), "\"内容 \"");
    }
}
