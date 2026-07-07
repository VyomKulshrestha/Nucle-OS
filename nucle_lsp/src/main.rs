//! NucleScript language server entry point.
//!
//! A thin protocol adapter over `nucle_lang::analyze` -- see `backend.rs`.
//! No compiler logic lives here or in `backend.rs`; every diagnostic,
//! hover, and definition answer comes from the exact same `check_source`/
//! `SymbolTable` data `nucle check` and the playground already produce, so
//! the LSP can never disagree with the CLI about what's wrong with a file.

use nucle_lsp::backend::Backend;
use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
