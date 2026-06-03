//! `mui-lsp` — the language server for MUI (`.mui` / `.crm`).
//!
//! Speaks LSP (JSON-RPC over stdio). Provides diagnostics, completion (widgets,
//! props, enum members, imported components), hover, document symbols, color
//! swatches, and go-to-definition for imports — all driven by `mui-syntax` (the
//! real parser + import loader) so the editor never drifts from the compiler.

use tower_lsp::{LspService, Server};

mod catalog;
mod docs;
mod server;

#[tokio::main]
async fn main() {
    // LSP is JSON-RPC over stdio; logs must go to stderr so they don't corrupt
    // the protocol stream on stdout.
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(server::Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
