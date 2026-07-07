//! Library half of `nucle_lsp`, split out from the `nucle-lsp` binary
//! purely so integration tests (`tests/`) can drive a real `Backend`
//! instance over an in-memory transport instead of only unit-testing its
//! internal functions.

pub mod backend;
