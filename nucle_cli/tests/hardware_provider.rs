//! CLI-level vendor integration tests. Every network request targets a
//! loopback stand-in; no live vendor host or credential is used.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);
const PURE_PROGRAM: &str = "../docs/examples/probabilistic_recovery.nsl";
const EFFECTFUL_PROGRAM: &str = "../docs/examples/effect_confirmations.nsl";

fn nucle_cli_bin() -> &'static str {
    env!("CARGO_BIN_EXE_nucle-cli")
}

fn scratch_dir(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "nucle_cli_hardware_provider_{name}_{}_{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn run_export(
    source: &Path,
    provider: &str,
    config: &Path,
    job_file: &Path,
    credentials: &[(&str, &str)],
    confirm: bool,
) -> Output {
    let mut command = Command::new(nucle_cli_bin());
    command
        .arg("hardware")
        .arg("export")
        .arg(source)
        .arg("--provider")
        .arg(provider)
        .arg("--config")
        .arg(config)
        .arg("--job-file")
        .arg(job_file);
    if confirm {
        command.arg("--confirm");
    }
    for (name, value) in credentials {
        command.env(name, value);
    }
    command.output().expect("failed to spawn nucle-cli export")
}

fn write_synthesis_program(dir: &Path) -> PathBuf {
    let path = dir.join("synthesis.nsl");
    std::fs::write(
        &path,
        "pool archive: DnaPool { codec: Ternary, redundancy: 3x, profile: Twist }\nlet strands: Pool<Twist, 0.03%> = synthesise archive via Twist confirm hardware\n",
    )
    .unwrap();
    path
}

fn write_sequencing_program(dir: &Path, profile: &str) -> PathBuf {
    let path = dir.join("sequencing.nsl");
    let error_rate = if profile == "Illumina" { "0.35%" } else { "5%" };
    std::fs::write(
        &path,
        format!(
            "pool strands: DnaPool {{ codec: Ternary, redundancy: 3x, profile: Twist }}\nlet reads: Pool<{profile}, {error_rate}> = sequence strands via {profile} confirm hardware\n"
        ),
    )
    .unwrap();
    path
}

fn run_status(config: &Path, job_file: &Path, credentials: &[(&str, &str)]) -> Output {
    let mut command = Command::new(nucle_cli_bin());
    command
        .arg("--json")
        .arg("hardware")
        .arg("status")
        .arg(job_file)
        .arg("--config")
        .arg(config);
    for (name, value) in credentials {
        command.env(name, value);
    }
    command.output().expect("failed to spawn nucle-cli status")
}

fn spawn_json_server(
    requests: usize,
    responder: impl Fn(&str, &str) -> String + Send + 'static,
) -> String {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}", server.server_addr());
    std::thread::spawn(move || {
        for _ in 0..requests {
            let mut request = server.recv().unwrap();
            let url = request.url().to_string();
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            let response = tiny_http::Response::from_string(responder(&url, &body)).with_header(
                "Content-Type: application/json"
                    .parse::<tiny_http::Header>()
                    .unwrap(),
            );
            request.respond(response).unwrap();
        }
    });
    base_url
}

#[test]
fn unknown_provider_is_rejected_before_any_output_is_created() {
    let dir = scratch_dir("unknown");
    let output = dir.join("batch.json");
    let result = Command::new(nucle_cli_bin())
        .arg("hardware")
        .arg("export")
        .arg(PURE_PROGRAM)
        .arg("--provider")
        .arg("typo-provider")
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();

    assert!(!result.status.success());
    assert!(!output.exists());
    assert!(String::from_utf8_lossy(&result.stderr)
        .contains("provider 'typo-provider' is not available"));
}

#[test]
fn vendor_submission_preserves_the_operator_confirmation_gate() {
    let dir = scratch_dir("confirm");
    let config = dir.join("twist.json");
    let job = dir.join("job.jsonl");
    std::fs::write(
        &config,
        r#"{"provider":"twist","base_url":"https://example.invalid","order_path":"/orders","status_path_template":"/orders/{id}","email_env":"TEST_TWIST_EMAIL","api_token_env":"TEST_TWIST_TOKEN"}"#,
    )
    .unwrap();

    let output = run_export(
        Path::new(EFFECTFUL_PROGRAM),
        "twist",
        &config,
        &job,
        &[
            ("TEST_TWIST_EMAIL", "lab@example.com"),
            ("TEST_TWIST_TOKEN", "dummy"),
        ],
        false,
    );
    assert!(!output.status.success());
    assert!(
        !job.exists(),
        "confirmation refusal must happen before job-file creation"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("without --confirm"));
}

#[test]
fn vendor_config_with_unknown_fields_fails_closed() {
    let dir = scratch_dir("unknown_field");
    let config = dir.join("twist.json");
    let job = dir.join("job.jsonl");
    std::fs::write(
        &config,
        r#"{"provider":"twist","base_url":"https://example.invalid","order_path":"/orders","status_path_template":"/orders/{id}","email_env":"TEST_TWIST_EMAIL","api_token_env":"TEST_TWIST_TOKEN","allow_insecure":true}"#,
    )
    .unwrap();

    let output = run_export(
        Path::new(EFFECTFUL_PROGRAM),
        "twist",
        &config,
        &job,
        &[
            ("TEST_TWIST_EMAIL", "lab@example.com"),
            ("TEST_TWIST_TOKEN", "dummy"),
        ],
        true,
    );
    assert!(!output.status.success());
    assert!(!job.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown field"));
}

#[test]
fn mixed_vendor_batch_is_rejected_before_job_creation_or_network_access() {
    let dir = scratch_dir("mixed_batch");
    let config = dir.join("twist.json");
    let job = dir.join("job.jsonl");
    std::fs::write(
        &config,
        r#"{"provider":"twist","base_url":"https://example.invalid","order_path":"/orders","status_path_template":"/orders/{id}","email_env":"TEST_TWIST_EMAIL","api_token_env":"TEST_TWIST_TOKEN"}"#,
    )
    .unwrap();

    let output = run_export(
        Path::new(EFFECTFUL_PROGRAM),
        "twist",
        &config,
        &job,
        &[
            ("TEST_TWIST_EMAIL", "lab@example.com"),
            ("TEST_TWIST_TOKEN", "dummy"),
        ],
        true,
    );
    assert!(!output.status.success());
    assert!(!job.exists());
    assert!(String::from_utf8_lossy(&output.stderr).contains("provider-scoped source file"));
}

#[test]
fn twist_export_and_status_resume_against_a_loopback_server() {
    let base_url = spawn_json_server(2, |url, _| {
        if url == "/orders" {
            r#"{"id":"twist-123"}"#.to_string()
        } else {
            r#"{"status":"complete"}"#.to_string()
        }
    });
    let dir = scratch_dir("twist");
    let source = write_synthesis_program(&dir);
    let config = dir.join("config.json");
    let job = dir.join("job.jsonl");
    std::fs::write(
        &config,
        serde_json::json!({
            "provider": "twist",
            "base_url": base_url,
            "order_path": "/orders",
            "status_path_template": "/orders/{id}",
            "email_env": "TEST_TWIST_EMAIL",
            "api_token_env": "TEST_TWIST_TOKEN"
        })
        .to_string(),
    )
    .unwrap();
    let credentials = [
        ("TEST_TWIST_EMAIL", "lab@example.com"),
        ("TEST_TWIST_TOKEN", "dummy-token"),
    ];

    let submitted = run_export(&source, "twist", &config, &job, &credentials, true);
    assert!(
        submitted.status.success(),
        "{}",
        String::from_utf8_lossy(&submitted.stderr)
    );
    assert!(job.exists());
    assert!(!std::fs::read_to_string(&job)
        .unwrap()
        .contains("dummy-token"));

    let status = run_status(&config, &job, &credentials);
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(String::from_utf8_lossy(&status.stdout).contains(r#""status": "Complete""#));
}

#[test]
fn idt_export_and_status_resume_against_a_loopback_server() {
    let base_url = spawn_json_server(4, |url, body| {
        if url == "/token" {
            r#"{"access_token":"fake-bearer"}"#.to_string()
        } else if body.contains("items") {
            r#"{"id":"idt-456"}"#.to_string()
        } else {
            r#"{"status":"shipped"}"#.to_string()
        }
    });
    let dir = scratch_dir("idt");
    let source = write_synthesis_program(&dir);
    let config = dir.join("config.json");
    let job = dir.join("job.jsonl");
    std::fs::write(
        &config,
        serde_json::json!({
            "provider": "idt",
            "client_id_env": "TEST_IDT_CLIENT_ID",
            "client_secret_env": "TEST_IDT_CLIENT_SECRET",
            "username_env": "TEST_IDT_USERNAME",
            "password_env": "TEST_IDT_PASSWORD",
            "token_url": format!("{base_url}/token"),
            "api_base_url": base_url,
            "order_path": "/orders",
            "status_path_template": "/orders/{id}"
        })
        .to_string(),
    )
    .unwrap();
    let credentials = [
        ("TEST_IDT_CLIENT_ID", "dummy-client"),
        ("TEST_IDT_CLIENT_SECRET", "dummy-secret"),
        ("TEST_IDT_USERNAME", "user@example.com"),
        ("TEST_IDT_PASSWORD", "dummy-password"),
    ];

    let submitted = run_export(&source, "idt", &config, &job, &credentials, true);
    assert!(
        submitted.status.success(),
        "{}",
        String::from_utf8_lossy(&submitted.stderr)
    );
    let status = run_status(&config, &job, &credentials);
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(String::from_utf8_lossy(&status.stdout).contains(r#""status": "Complete""#));
}

#[test]
fn illumina_export_and_status_resume_against_a_loopback_server() {
    let base_url = spawn_json_server(2, |url, _| {
        if url.contains("launch") {
            r#"{"Id":"session-789"}"#.to_string()
        } else {
            r#"{"ExecutionStatus":"Running"}"#.to_string()
        }
    });
    let dir = scratch_dir("illumina");
    let source = write_sequencing_program(&dir, "Illumina");
    let config = dir.join("config.json");
    let job = dir.join("job.jsonl");
    std::fs::write(
        &config,
        serde_json::json!({
            "provider": "illumina",
            "base_url": base_url,
            "access_token_env": "TEST_ILLUMINA_TOKEN",
            "launch_path": "/applications/app-1/launch"
        })
        .to_string(),
    )
    .unwrap();
    let credentials = [("TEST_ILLUMINA_TOKEN", "dummy-token")];

    let submitted = run_export(&source, "illumina", &config, &job, &credentials, true);
    assert!(
        submitted.status.success(),
        "{}",
        String::from_utf8_lossy(&submitted.stderr)
    );
    let status = run_status(&config, &job, &credentials);
    assert!(
        status.status.success(),
        "{}",
        String::from_utf8_lossy(&status.stderr)
    );
    assert!(String::from_utf8_lossy(&status.stdout).contains(r#""status": "Running""#));
}

#[test]
fn nanopore_missing_credential_fails_closed_before_connecting() {
    let dir = scratch_dir("nanopore");
    let config = dir.join("config.json");
    let ca = dir.join("ca.pem");
    let job = dir.join("job.jsonl");
    std::fs::write(
        &ca,
        "-----BEGIN CERTIFICATE-----\nTEST\n-----END CERTIFICATE-----\n",
    )
    .unwrap();
    std::fs::write(
        &config,
        serde_json::json!({
            "provider": "nanopore",
            "host": "127.0.0.1",
            "manager_port": 9502,
            "ca_certificate_path": "ca.pem",
            "protocol_identifier": "sequencing/test",
            "auth": { "type": "developer-api-token", "token_env": "TEST_NANOPORE_MISSING_TOKEN" }
        })
        .to_string(),
    )
    .unwrap();

    let output = run_export(
        Path::new(EFFECTFUL_PROGRAM),
        "nanopore",
        &config,
        &job,
        &[],
        true,
    );
    assert!(!output.status.success());
    assert!(
        !job.exists(),
        "credential validation must finish before job-file creation"
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("is not set"));
}
