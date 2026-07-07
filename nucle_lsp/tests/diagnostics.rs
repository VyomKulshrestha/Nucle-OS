//! Integration test driving `nucle_lsp` over the real LSP wire protocol
//! (Content-Length-framed JSON-RPC on an in-memory duplex pipe), not just
//! calling its internal functions directly -- this is what actually
//! proves the server behaves correctly for a real editor, not just that
//! its Rust functions compose correctly.

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tower_lsp::{LspService, Server};

async fn write_message(stream: &mut DuplexStream, value: Value) {
    let body = serde_json::to_string(&value).unwrap();
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    stream.write_all(header.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
}

/// Read exactly one Content-Length-framed JSON-RPC message, skipping any
/// server->client requests/notifications that don't match `predicate`
/// (e.g. the `window/logMessage` notification sent from `initialized`)
/// so the test isn't coupled to the exact order of unrelated messages.
async fn read_message_matching(stream: &mut DuplexStream, predicate: impl Fn(&Value) -> bool) -> Value {
    loop {
        let mut header = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).await.unwrap();
            header.push(byte[0]);
            if header.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let header_str = String::from_utf8(header).unwrap();
        let content_length: usize = header_str
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .expect("expected Content-Length header")
            .trim()
            .parse()
            .unwrap();

        let mut body = vec![0u8; content_length];
        stream.read_exact(&mut body).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        if predicate(&value) {
            return value;
        }
    }
}

/// Spin up a real `Backend` server on one end of an in-memory duplex
/// pipe, returning the other end for the test to speak raw LSP to.
fn start_server() -> DuplexStream {
    let (client_stream, server_stream) = tokio::io::duplex(1 << 16);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (service, socket) = LspService::new(nucle_lsp::backend::Backend::new);
    tokio::spawn(async move {
        Server::new(server_read, server_write, socket).serve(service).await;
    });
    client_stream
}

#[tokio::test]
async fn publishes_diagnostics_matching_nucle_check_for_a_broken_program() {
    let mut client = start_server();

    write_message(
        &mut client,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
    )
    .await;
    let init_response = read_message_matching(&mut client, |v| v.get("id") == Some(&json!(1))).await;
    assert!(init_response.get("result").is_some(), "expected a successful initialize response, got: {init_response:?}");

    write_message(&mut client, json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} })).await;

    let broken_source = "\n        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist }\n        delete \"old_archive.bin\" from archive\n    ";
    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///broken.nsl",
                    "languageId": "nuclescript",
                    "version": 1,
                    "text": broken_source
                }
            }
        }),
    )
    .await;

    let publish = read_message_matching(&mut client, |v| v.get("method") == Some(&json!("textDocument/publishDiagnostics"))).await;
    let diagnostics = publish["params"]["diagnostics"].as_array().expect("diagnostics array");
    assert_eq!(diagnostics.len(), 1, "expected exactly one diagnostic, got: {diagnostics:?}");

    let diagnostic = &diagnostics[0];
    assert_eq!(diagnostic["code"], json!("E-DELETE-UNCONFIRMED"));
    assert_eq!(diagnostic["severity"], json!(1), "severity 1 == Error in the LSP spec");
    assert!(
        diagnostic["message"].as_str().unwrap().contains("requires explicit physical key confirmation"),
        "unexpected message: {:?}", diagnostic["message"]
    );

    // Cross-check directly against nucle_check's own analysis of the exact
    // same source -- this is the property that actually matters: the LSP
    // must never disagree with the CLI about what's wrong with a file.
    let cli_report = nucle_lang::check_source(broken_source);
    assert_eq!(cli_report.diagnostics.len(), 1);
    assert_eq!(cli_report.diagnostics[0].code, "E-DELETE-UNCONFIRMED");

    // LSP positions are 0-indexed; nucle_lang's spans are 1-indexed.
    let expected_line = cli_report.diagnostics[0].span.line - 1;
    let expected_col = cli_report.diagnostics[0].span.column - 1;
    assert_eq!(diagnostic["range"]["start"]["line"], json!(expected_line));
    assert_eq!(diagnostic["range"]["start"]["character"], json!(expected_col));
}

#[tokio::test]
async fn publishes_no_diagnostics_for_a_valid_program() {
    let mut client = start_server();

    write_message(
        &mut client,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
    )
    .await;
    read_message_matching(&mut client, |v| v.get("id") == Some(&json!(1))).await;
    write_message(&mut client, json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} })).await;

    let valid_source = "\n        pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }\n        simulate store \"README.md\" into archive { redundancy: 4x, coverage: 4x }\n    ";
    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": "file:///valid.nsl",
                    "languageId": "nuclescript",
                    "version": 1,
                    "text": valid_source
                }
            }
        }),
    )
    .await;

    let publish = read_message_matching(&mut client, |v| v.get("method") == Some(&json!("textDocument/publishDiagnostics"))).await;
    let diagnostics = publish["params"]["diagnostics"].as_array().expect("diagnostics array");
    assert!(diagnostics.is_empty(), "expected no diagnostics for a valid program, got: {diagnostics:?}");
}

/// 0-indexed (line, character) of the first occurrence of `needle` in
/// `text` -- computed rather than hand-counted, so a test fixture edit
/// can't silently desync the position from the source it's testing.
fn find_position(text: &str, needle: &str) -> (u32, u32) {
    for (line_no, line) in text.lines().enumerate() {
        if let Some(byte_offset) = line.find(needle) {
            let char_offset = line[..byte_offset].chars().count();
            return (line_no as u32, char_offset as u32);
        }
    }
    panic!("'{needle}' not found in fixture source");
}

#[tokio::test]
async fn hover_and_definition_resolve_a_pool_reference() {
    let mut client = start_server();

    write_message(
        &mut client,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
    )
    .await;
    read_message_matching(&mut client, |v| v.get("id") == Some(&json!(1))).await;
    write_message(&mut client, json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} })).await;

    let source = "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }\nsimulate store \"README.md\" into archive { redundancy: 4x, coverage: 4x }\n";
    let uri = "file:///hover.nsl";
    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": uri, "languageId": "nuclescript", "version": 1, "text": source }
            }
        }),
    )
    .await;
    read_message_matching(&mut client, |v| v.get("method") == Some(&json!("textDocument/publishDiagnostics"))).await;

    // Hover over the pool *declaration* itself.
    let (decl_line, decl_char) = find_position(source, "archive");
    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "textDocument/hover",
            "params": { "textDocument": { "uri": uri }, "position": { "line": decl_line, "character": decl_char + 1 } }
        }),
    )
    .await;
    let hover = read_message_matching(&mut client, |v| v.get("id") == Some(&json!(2))).await;
    let hover_text = hover["result"]["contents"].as_str().expect("hover contents should be a string");
    assert!(hover_text.contains("pool archive"), "unexpected hover text: {hover_text:?}");
    assert!(hover_text.contains("Illumina"), "unexpected hover text: {hover_text:?}");

    // Go to definition from the *usage* site on the second line.
    let (use_line, use_char) = {
        let second_line_offset = source.find('\n').unwrap() + 1;
        let (line, char_on_line) = find_position(&source[second_line_offset..], "archive");
        (line + 1, char_on_line) // +1 because we searched starting from the second line
    };
    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "textDocument/definition",
            "params": { "textDocument": { "uri": uri }, "position": { "line": use_line, "character": use_char + 1 } }
        }),
    )
    .await;
    let definition = read_message_matching(&mut client, |v| v.get("id") == Some(&json!(3))).await;
    assert_eq!(definition["result"]["uri"], json!(uri));
    assert_eq!(definition["result"]["range"]["start"]["line"], json!(decl_line));
}
