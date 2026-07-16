//! Proves the Prometheus metrics exporter end to end: `/metrics` reports
//! real pool state and re-reads it fresh on every scrape (a file stored
//! *while the server is running* shows up on the very next scrape, no
//! restart needed), `/` and unknown paths respond sensibly, and a
//! non-`GET` method is rejected. Spawns the real, compiled `nucle-cli`
//! binary (`CARGO_BIN_EXE_nucle-cli`) as a genuine long-running child
//! process (`Command::spawn`, not `.output()`, since `serve` blocks
//! forever) and talks to it over a raw `TcpStream` -- no HTTP-client
//! dependency needed for one plain GET.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("nucle_cli_serve_test_{}_{}", name, std::process::id()))
}

struct ServerGuard(Child);

impl Drop for ServerGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_server(pool_dir: &PathBuf, port: u16) -> ServerGuard {
    let child = Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(pool_dir)
        .arg("serve").arg("--port").arg(port.to_string())
        .spawn()
        .expect("failed to spawn nucle-cli serve");
    ServerGuard(child)
}

/// A minimal raw-HTTP/1.1 GET, since this is the one request shape every
/// test here needs and pulling in a real HTTP client for that would be
/// more dependency than the job requires.
fn http_get(port: u16, path: &str, method: &str) -> (u32, String) {
    let deadline = Instant::now() + Duration::from_secs(5);
    let stream = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(s) => break s,
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => panic!("could not connect to server on port {}: {}", port, e),
        }
    };
    let mut stream = stream;
    let request = format!("{} {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n", method, path);
    stream.write_all(request.as_bytes()).unwrap();

    let mut raw = String::new();
    stream.read_to_string(&mut raw).unwrap();

    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status_line = head.lines().next().unwrap_or("");
    let status: u32 = status_line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (status, body.to_string())
}

#[test]
fn metrics_endpoint_reports_real_pool_state() {
    let pool_dir = scratch_dir("metrics_pool");
    let src_dir = scratch_dir("metrics_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let file = src_dir.join("a.txt");
    std::fs::write(&file, b"metrics test data").unwrap();
    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file)
        .arg("--redundancy").arg("1")
        .output()
        .expect("failed to spawn nucle-cli for store");

    let port = 19901;
    let _server = start_server(&pool_dir, port);

    let (status, body) = http_get(port, "/metrics", "GET");
    assert_eq!(status, 200);
    assert!(body.contains("nucleos_pool_files 1"));
    assert!(body.contains("# TYPE nucleos_pool_files gauge"));
    assert!(body.contains("nucleos_audit_events_total{operation=\"write\",success=\"true\"} 1"));

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn metrics_are_re_read_fresh_on_every_scrape_without_restarting() {
    let pool_dir = scratch_dir("live_pool");
    let src_dir = scratch_dir("live_src");
    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).unwrap();

    let port = 19902;
    let _server = start_server(&pool_dir, port);

    let (_, before) = http_get(port, "/metrics", "GET");
    assert!(before.contains("nucleos_pool_files 0"));

    let file = src_dir.join("b.txt");
    std::fs::write(&file, b"stored while the server is already running").unwrap();
    Command::new(nucle_cli_bin())
        .arg("--pool-dir").arg(&pool_dir)
        .arg("store").arg(&file)
        .arg("--redundancy").arg("0")
        .output()
        .expect("failed to spawn nucle-cli for store");

    let (_, after) = http_get(port, "/metrics", "GET");
    assert!(after.contains("nucleos_pool_files 1"), "expected the live store to show up without restarting the server, got: {}", after);

    let _ = std::fs::remove_dir_all(&pool_dir);
    let _ = std::fs::remove_dir_all(&src_dir);
}

#[test]
fn root_and_unknown_paths_and_wrong_methods_respond_sensibly() {
    let pool_dir = scratch_dir("routing_pool");
    let _ = std::fs::remove_dir_all(&pool_dir);

    let port = 19903;
    let _server = start_server(&pool_dir, port);

    let (root_status, root_body) = http_get(port, "/", "GET");
    assert_eq!(root_status, 200);
    assert!(root_body.contains("metrics exporter"));

    let (missing_status, _) = http_get(port, "/does-not-exist", "GET");
    assert_eq!(missing_status, 404);

    let (wrong_method_status, _) = http_get(port, "/metrics", "POST");
    assert_eq!(wrong_method_status, 405);

    let _ = std::fs::remove_dir_all(&pool_dir);
}
