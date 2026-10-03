use std::collections::HashMap;

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

use crate::item::{Ctor, Item, Value};

/// Run the language server over stdio.
pub async fn serve() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend {
        client,
        documents: tokio::sync::Mutex::new(HashMap::new()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}

struct Backend {
    client: Client,
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
    }

    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
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
        let Some((src, item)) = self.analyzed(uri).await else {
            return Ok(None);
        };
        let lines = LineIndex::new(&src);
        let offset = lines.offset(&src, position);
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
            let index = crate::index::Index::build(&item, &mut Vec::new());
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
        let target_path = crate::vault::normalize(&dir.join(path_part));
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
        let (_item, diagnostics) = crate::analyze(&text);
        let lines = LineIndex::new(&text);
        let diagnostics = diagnostics
            .iter()
            .map(|d| Diagnostic {
                range: lines.range(&text, d.span),
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some(format!("notist:{}", d.phase)),
                message: d.message.clone(),
                ..Default::default()
            })
            .collect();
        self.client
            .publish_diagnostics(uri, diagnostics, None)
            .await;
    }

    async fn analyzed(&self, uri: &Url) -> Option<(String, Item)> {
        let src = self.documents.lock().await.get(uri)?.clone();
        let (item, _) = crate::analyze(&src);
        Some((src, item))
    }
}

/// The smallest item whose span contains `offset`.
fn smallest_at(item: &Item, offset: usize) -> Option<&Item> {
    for child in &item.children {
        if child.span.contains(rowan::TextSize::from(offset as u32)) {
            return Some(smallest_at(child, offset).unwrap_or(child));
        }
    }
    None
}

fn hover_text(item: &Item) -> String {
    let mut out = format!("`{}`", item.ctor.name().to_lowercase());
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

    fn range(&self, src: &str, span: rowan::TextRange) -> Range {
        Range::new(
            self.position(src, usize::from(span.start())),
            self.position(src, usize::from(span.end())),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smallest_at_finds_innermost() {
        let (item, _) = crate::analyze("内容 *粗体* 文字\n");
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
