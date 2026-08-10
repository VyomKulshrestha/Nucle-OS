//! End-to-end protocol coverage for the editor features advertised by
//! `nucle_lsp`. These tests use framed JSON-RPC over an in-memory stream,
//! exactly as an editor client does.

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tower_lsp::{LspService, Server};

async fn write_message(stream: &mut DuplexStream, value: Value) {
    let body = serde_json::to_string(&value).unwrap();
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    stream.write_all(header.as_bytes()).await.unwrap();
    stream.write_all(body.as_bytes()).await.unwrap();
}

async fn read_message_matching(
    stream: &mut DuplexStream,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
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
        let header = String::from_utf8(header).unwrap();
        let content_length = header
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .unwrap()
            .trim()
            .parse::<usize>()
            .unwrap();
        let mut body = vec![0u8; content_length];
        stream.read_exact(&mut body).await.unwrap();
        let value: Value = serde_json::from_slice(&body).unwrap();
        if predicate(&value) {
            return value;
        }
    }
}

fn start_server() -> DuplexStream {
    let (client_stream, server_stream) = tokio::io::duplex(1 << 16);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (service, socket) = LspService::new(nucle_lsp::backend::Backend::new);
    tokio::spawn(async move {
        Server::new(server_read, server_write, socket)
            .serve(service)
            .await;
    });
    client_stream
}

fn position_of(source: &str, needle: &str, occurrence: usize) -> Value {
    let byte_offset = source
        .match_indices(needle)
        .nth(occurrence)
        .map(|(offset, _)| offset)
        .unwrap();
    let before = &source[..byte_offset];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |offset| offset + 1);
    let character = source[line_start..byte_offset].encode_utf16().count();
    json!({ "line": line, "character": character })
}

#[tokio::test]
async fn serves_completion_references_rename_and_semantic_tokens() {
    let mut client = start_server();
    write_message(
        &mut client,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "capabilities": {} } }),
    )
    .await;
    let initialized =
        read_message_matching(&mut client, |value| value.get("id") == Some(&json!(1))).await;
    let capabilities = &initialized["result"]["capabilities"];
    assert!(capabilities["completionProvider"].is_object());
    assert_eq!(capabilities["referencesProvider"], json!(true));
    assert_eq!(
        capabilities["renameProvider"]["prepareProvider"],
        json!(true)
    );
    assert!(capabilities["semanticTokensProvider"].is_object());

    write_message(
        &mut client,
        json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;
    let uri = "file:///editor-features.nsl";
    let source = "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Illumina }\n\
                  simulate store \"README.md\" into archive { redundancy: 4x, coverage: 4x }\n";
    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": {
                    "uri": uri,
                    "languageId": "nuclescript",
                    "version": 1,
                    "text": source
                }
            }
        }),
    )
    .await;
    read_message_matching(&mut client, |value| {
        value.get("method") == Some(&json!("textDocument/publishDiagnostics"))
    })
    .await;

    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "textDocument/completion",
            "params": { "textDocument": { "uri": uri }, "position": { "line": 1, "character": 0 } }
        }),
    )
    .await;
    let completion =
        read_message_matching(&mut client, |value| value.get("id") == Some(&json!(2))).await;
    let labels = completion["result"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["label"].as_str())
        .collect::<Vec<_>>();
    assert!(labels.contains(&"archive"));
    assert!(labels.contains(&"consensus_vote"));

    let use_position = position_of(source, "archive", 1);
    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "textDocument/references",
            "params": {
                "textDocument": { "uri": uri }, "position": use_position,
                "context": { "includeDeclaration": true }
            }
        }),
    )
    .await;
    let references =
        read_message_matching(&mut client, |value| value.get("id") == Some(&json!(3))).await;
    assert_eq!(references["result"].as_array().unwrap().len(), 2);

    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 4, "method": "textDocument/prepareRename",
            "params": { "textDocument": { "uri": uri }, "position": use_position }
        }),
    )
    .await;
    let prepare =
        read_message_matching(&mut client, |value| value.get("id") == Some(&json!(4))).await;
    assert_eq!(prepare["result"]["placeholder"], json!("archive"));

    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 5, "method": "textDocument/rename",
            "params": {
                "textDocument": { "uri": uri }, "position": use_position, "newName": "cold_archive"
            }
        }),
    )
    .await;
    let rename =
        read_message_matching(&mut client, |value| value.get("id") == Some(&json!(5))).await;
    let edits = rename["result"]["changes"][uri].as_array().unwrap();
    assert_eq!(edits.len(), 2);
    assert!(edits
        .iter()
        .all(|edit| edit["newText"] == json!("cold_archive")));

    write_message(
        &mut client,
        json!({
            "jsonrpc": "2.0", "id": 6, "method": "textDocument/semanticTokens/full",
            "params": { "textDocument": { "uri": uri } }
        }),
    )
    .await;
    let semantic =
        read_message_matching(&mut client, |value| value.get("id") == Some(&json!(6))).await;
    let data = semantic["result"]["data"].as_array().unwrap();
    assert!(!data.is_empty());
    assert_eq!(
        data.len() % 5,
        0,
        "semantic token wire data must contain five integers per token"
    );
}
