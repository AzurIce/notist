use std::collections::HashMap;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, InitializeParams, InitializeResult, InitializedParams, Position,
    Range, ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use tower_lsp::{Client, LanguageServer, LspService, Server};

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
}

impl Backend {
    async fn on_change(&self, uri: Url, text: String) {
        self.documents.lock().await.insert(uri.clone(), text.clone());
        let (_item, diagnostics) = crate::analyze(&text);
        let lines = LineIndex::new(&text);
        let diagnostics = diagnostics
            .iter()
            .map(|d| {
                let start = usize::from(d.span.start());
                let end = usize::from(d.span.end());
                Diagnostic {
                    range: Range::new(lines.position(&text, start), lines.position(&text, end)),
                    severity: Some(DiagnosticSeverity::ERROR),
                    source: Some("notist".to_string()),
                    message: d.message.clone(),
                    ..Default::default()
                }
            })
            .collect();
        self.client.publish_diagnostics(uri, diagnostics, None).await;
    }
}

/// Byte offset → LSP position (line + UTF-16 column).
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
}
