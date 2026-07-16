//! A real HTTP client against Illumina's BaseSpace Sequence Hub v2 REST API.
//!
//! Confirmed from Illumina's public developer portal
//! (`developer.basespace.illumina.com`): the v2 API is reached under
//! `/v2` on a BaseSpace host, authenticates via an `x-access-token`
//! header (or full OAuth2), and represents a running job as an
//! `AppSession` resource -- `GET /v2/appsessions/{id}` returns its
//! `ExecutionStatus` (`Running`, `Complete`, `TimedOut`, `Aborted`,
//! `NeedsAttention`), and `POST /v2/appsessions/{id}` updates one.
//!
//! What matters for this project's domain, and what's worth being honest
//! about: BaseSpace doesn't offer a "mail order sequencing" API the way
//! Twist/IDT offer synthesis ordering -- physical sequencing always
//! happens on an owned instrument (the same situation as Oxford Nanopore,
//! just wrapped differently by Illumina's ecosystem). What BaseSpace
//! *does* expose over the network is submitting/monitoring a secondary
//! analysis app against already-sequenced data, which is what `submit()`
//! below does: it launches an application (creating a new `AppSession`)
//! rather than commissioning a wet-lab run. The exact per-application
//! launch endpoint/body is application-specific and isn't confirmed
//! generically here (BaseSpace's public docs describe *updating* an
//! existing AppSession in detail but not the generic launch call), so
//! `IlluminaConfig` requires it explicitly rather than guessing.
//!
//! Untested against a live BaseSpace account -- there are no BaseSpace
//! credentials available in this project's development environment -- but
//! every request below is a genuine HTTP call, not a mock.

use crate::provider::{ImmediateJobHandle, JobHandle, JobStatus, Provider};
use nucle_lang::hardware::HardwareRequest;
use serde::{Deserialize, Serialize};

pub struct IlluminaConfig {
    /// BaseSpace's v2 API base URL. Defaults to the real, public US host
    /// (`https://api.basespace.illumina.com/v2`) -- override for a
    /// region-specific deployment (e.g. `api.euc1.sh.basespace.illumina.com`).
    pub base_url: String,
    /// An OAuth2 access token or developer API key, sent as `x-access-token`.
    pub access_token: String,
    /// Path (relative to `base_url`) that launches a specific application
    /// and creates a new `AppSession` -- e.g. `"/applications/{app_id}/launch"`.
    /// Application-specific and not confirmed generically here; required
    /// from the caller rather than defaulted.
    pub launch_path: String,
}

impl IlluminaConfig {
    pub fn new(access_token: impl Into<String>, launch_path: impl Into<String>) -> Self {
        Self {
            base_url: "https://api.basespace.illumina.com/v2".to_string(),
            access_token: access_token.into(),
            launch_path: launch_path.into(),
        }
    }
}

#[derive(Serialize)]
struct LaunchRequest {
    inputs: Vec<LaunchInput>,
}

#[derive(Serialize)]
struct LaunchInput {
    name: String,
}

#[derive(Deserialize)]
struct AppSessionCreated {
    #[serde(rename = "Id")]
    id: String,
}

#[derive(Deserialize)]
struct AppSessionStatus {
    #[serde(rename = "ExecutionStatus")]
    execution_status: String,
}

/// A real HTTP `Provider` for Illumina's BaseSpace Sequence Hub v2 API.
/// See the module doc comment for what "real" does and doesn't mean here,
/// and for the AppSession-vs-physical-sequencing distinction.
pub struct IlluminaProvider {
    config: IlluminaConfig,
    agent: ureq::Agent,
}

impl IlluminaProvider {
    pub fn new(config: IlluminaConfig) -> Self {
        Self { config, agent: crate::http_client::new_agent() }
    }
}

/// Classifies BaseSpace's real, documented `ExecutionStatus` values.
/// Unlike Twist/IDT's status vocabulary (genuinely unconfirmed), these
/// five values (`Running`/`Complete`/`TimedOut`/`Aborted`/
/// `NeedsAttention`) are Illumina's own documented enum -- still worth
/// double-checking against a live account, but not a guess in the same
/// way.
fn classify_execution_status(status: &str) -> JobStatus {
    match status {
        "Complete" => JobStatus::Complete(format!("BaseSpace AppSession status: {status}")),
        "TimedOut" | "Aborted" | "NeedsAttention" => {
            JobStatus::Failed(format!("BaseSpace AppSession status: {status}"))
        }
        _ => JobStatus::Running,
    }
}

impl Provider for IlluminaProvider {
    fn name(&self) -> &str {
        "illumina"
    }

    fn submit(&self, batch: &[HardwareRequest]) -> Box<dyn JobHandle> {
        let inputs = batch.iter().map(|request| LaunchInput { name: request.target.clone() }).collect();
        let body = LaunchRequest { inputs };

        let url = format!("{}{}", self.config.base_url, self.config.launch_path);
        let result = self
            .agent
            .post(&url)
            .header("x-access-token", &self.config.access_token)
            .send_json(&body)
            .map_err(|e| format!("BaseSpace app launch at {url} failed: {e}"))
            .and_then(|mut response| {
                response
                    .body_mut()
                    .read_json::<AppSessionCreated>()
                    .map_err(|e| format!("BaseSpace launch response from {url} was not valid JSON: {e}"))
            });

        match result {
            Ok(session) => Box::new(IlluminaJobHandle {
                status_url: format!("{}/appsessions/{}", self.config.base_url, session.id),
                access_token: self.config.access_token.clone(),
                agent: self.agent.clone(),
            }),
            Err(e) => Box::new(ImmediateJobHandle::new(Err(e))),
        }
    }
}

struct IlluminaJobHandle {
    status_url: String,
    access_token: String,
    agent: ureq::Agent,
}

impl JobHandle for IlluminaJobHandle {
    fn status(&self) -> JobStatus {
        let result = self
            .agent
            .get(&self.status_url)
            .header("x-access-token", &self.access_token)
            .call()
            .map_err(|e| format!("BaseSpace AppSession status poll of {} failed: {e}", self.status_url))
            .and_then(|mut response| {
                response
                    .body_mut()
                    .read_json::<AppSessionStatus>()
                    .map_err(|e| format!("BaseSpace AppSession status response from {} was not valid JSON: {e}", self.status_url))
            });

        match result {
            Ok(status) => classify_execution_status(&status.execution_status),
            Err(e) => JobStatus::Failed(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::sequencing_request;

    fn test_config(base_url: String) -> IlluminaConfig {
        let mut config = IlluminaConfig::new("test-access-token", "/applications/app-1/launch");
        config.base_url = base_url;
        config
    }

    /// A minimal local HTTP server standing in for BaseSpace, so the real
    /// launch/poll code runs end-to-end without touching Illumina's live
    /// API (which this crate has no credentials for).
    fn spawn_fake_basespace() -> (tiny_http::Server, String) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr();
        (server, format!("http://{addr}"))
    }

    fn handle_launch_then_status(server: tiny_http::Server, session_id: &str, status: &str, captured_headers: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>) {
        std::thread::spawn({
            let session_id = session_id.to_string();
            let status = status.to_string();
            move || {
                for _ in 0..2 {
                    let request = server.recv().unwrap();
                    for header in request.headers() {
                        captured_headers.lock().unwrap().push((header.field.to_string(), header.value.to_string()));
                    }
                    let response_body = if request.url().contains("launch") {
                        format!("{{\"Id\":\"{session_id}\"}}")
                    } else {
                        format!("{{\"ExecutionStatus\":\"{status}\"}}")
                    };
                    let response = tiny_http::Response::from_string(response_body)
                        .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap());
                    request.respond(response).unwrap();
                }
            }
        });
    }

    #[test]
    fn submit_launches_an_app_then_polls_its_appsession_status() {
        let (server, base_url) = spawn_fake_basespace();
        let headers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        handle_launch_then_status(server, "session-1", "Running", headers.clone());

        let provider = IlluminaProvider::new(test_config(base_url));
        let handle = provider.submit(&[sequencing_request("run_a.bin")]);
        let status = handle.status();

        assert_eq!(status, JobStatus::Running);
        let seen = headers.lock().unwrap();
        assert!(seen.iter().any(|(name, value)| name.eq_ignore_ascii_case("x-access-token") && value == "test-access-token"));
    }

    #[test]
    fn complete_execution_status_is_classified_as_complete() {
        let (server, base_url) = spawn_fake_basespace();
        handle_launch_then_status(server, "session-2", "Complete", std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));

        let provider = IlluminaProvider::new(test_config(base_url));
        let handle = provider.submit(&[sequencing_request("run_b.bin")]);
        assert!(matches!(handle.status(), JobStatus::Complete(_)));
    }

    #[test]
    fn aborted_execution_status_is_classified_as_failed() {
        let (server, base_url) = spawn_fake_basespace();
        handle_launch_then_status(server, "session-3", "Aborted", std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));

        let provider = IlluminaProvider::new(test_config(base_url));
        let handle = provider.submit(&[sequencing_request("run_c.bin")]);
        assert!(matches!(handle.status(), JobStatus::Failed(_)));
    }

    #[test]
    fn unreachable_base_url_fails_submit_cleanly() {
        let provider = IlluminaProvider::new(test_config("http://127.0.0.1:1".to_string()));
        let handle = provider.submit(&[sequencing_request("run_d.bin")]);
        assert!(matches!(handle.status(), JobStatus::Failed(_)));
    }

    #[test]
    fn provider_name_is_illumina() {
        let provider = IlluminaProvider::new(test_config("http://127.0.0.1:0".to_string()));
        assert_eq!(provider.name(), "illumina");
    }
}
