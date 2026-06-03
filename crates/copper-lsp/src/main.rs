use tower_lsp::{LspService, Server};

mod docs;
mod imports;
mod lexicon;
mod rust_prelude;
mod server;
mod stdlib_methods;

#[tokio::main]
async fn main() {
    // The LSP protocol speaks JSON-RPC over stdio. Logs go to stderr so
    // they don't corrupt the protocol stream on stdout.
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(server::Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
