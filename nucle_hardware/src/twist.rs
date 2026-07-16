//! A real HTTP client against Twist Bioscience's TAPI (Twist API) for gene
//! synthesis ordering.
//!
//! Confirmed from Twist's public developer portal
//! (`developers.twistdna.com/docs/tapi`): TAPI lets a customer place any
//! order programmatically that they could place through Twist's
//! eCommerce site via a `POST` request, and authenticates each request
//! with an API token tied to a single account email address (the token is
//! generated via a one-time email link). What is *not* independently
//! confirmed here -- because TAPI's full endpoint reference sits behind an
//! account-gated Swagger/ReadMe portal this crate's author has no Twist
//! account to view -- is the exact request/response JSON schema, the
//! precise header names, and the API's base hostname (as distinct from
//! `developers.twistdna.com`, which is the docs site, not the API itself).
//! `TwistConfig` therefore requires the base URL and endpoint paths
//! explicitly rather than guessing at defaults; the two header names have
//! defaults reflecting TAPI's documented email+token model but are
//! overridable in case the real names differ.
//!
//! Untested against a live Twist account -- there are no TAPI credentials
//! available in this project's development environment -- but every
//! request below is a genuine HTTP call, not a mock.

use crate::provider::{ImmediateJobHandle, JobHandle, JobStatus, Provider};
use nucle_lang::hardware::HardwareRequest;
use serde::{Deserialize, Serialize};

pub struct TwistConfig {
    /// TAPI's real base URL for your account -- distinct from the
    /// `developers.twistdna.com` docs site. Required rather than defaulted
    /// since it isn't public.
    pub base_url: String,
    /// Path (relative to `base_url`) that creates a new order, e.g.
    /// `"/orders"` -- verify against your account's TAPI reference.
    pub order_path: String,
    /// Path template for polling an order's status, with `{id}` replaced
    /// by the id TAPI returned from `order_path`, e.g. `"/orders/{id}"`.
    pub status_path_template: String,
    /// The account email address TAPI's docs say authentication is tied to.
    pub email: String,
    /// The one-time-email-link-generated API token.
    pub api_token: String,
    /// Header carrying `api_token`. Defaults to `"Authorization"` with a
    /// `Token <token>` value; override if your TAPI account's real header
    /// differs.
    pub auth_header_name: String,
    /// Header carrying `email`. Defaults to `"X-Twist-Email"`; override if
    /// your TAPI account's real header differs.
    pub email_header_name: String,
}

impl TwistConfig {
    pub fn new(base_url: impl Into<String>, order_path: impl Into<String>, status_path_template: impl Into<String>, email: impl Into<String>, api_token: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            order_path: order_path.into(),
            status_path_template: status_path_template.into(),
            email: email.into(),
            api_token: api_token.into(),
            auth_header_name: "Authorization".to_string(),
            email_header_name: "X-Twist-Email".to_string(),
        }
    }
}

#[derive(Serialize)]
struct TwistOrderRequest {
    name: String,
    constructs: Vec<TwistConstruct>,
}

#[derive(Serialize)]
struct TwistConstruct {
    name: String,
}

#[derive(Deserialize)]
struct TwistOrderResponse {
    id: String,
}

#[derive(Deserialize)]
struct TwistOrderStatusResponse {
    status: String,
}

/// A real HTTP `Provider` for Twist Bioscience's TAPI. See the module doc
/// comment for what "real" does and doesn't mean here.
pub struct TwistProvider {
    config: TwistConfig,
    agent: ureq::Agent,
}

impl TwistProvider {
    pub fn new(config: TwistConfig) -> Self {
        Self { config, agent: crate::http_client::new_agent() }
    }
}

/// Best-effort classification of TAPI's (unconfirmed) order-status
/// vocabulary -- see the module doc comment. Kept as a standalone function
/// so it's the one place to fix if the real vendor's wording differs.
fn classify_order_status(status: &str) -> JobStatus {
    let lower = status.to_ascii_lowercase();
    if ["complete", "completed", "shipped", "delivered", "fulfilled"]
        .iter()
        .any(|s| lower.contains(s))
    {
        JobStatus::Complete(format!("Twist order status: {status}"))
    } else if ["cancelled", "canceled", "failed", "error", "rejected"]
        .iter()
        .any(|s| lower.contains(s))
    {
        JobStatus::Failed(format!("Twist order status: {status}"))
    } else {
        JobStatus::Running
    }
}

impl Provider for TwistProvider {
    fn name(&self) -> &str {
        "twist"
    }

    fn submit(&self, batch: &[HardwareRequest]) -> Box<dyn JobHandle> {
        let constructs = batch
            .iter()
            .map(|request| TwistConstruct { name: request.target.clone() })
            .collect();
        let body = TwistOrderRequest { name: format!("NucleOS batch ({} constructs)", batch.len()), constructs };

        let url = format!("{}{}", self.config.base_url, self.config.order_path);
        let result = self
            .agent
            .post(&url)
            .header(&self.config.auth_header_name, format!("Token {}", self.config.api_token))
            .header(&self.config.email_header_name, &self.config.email)
            .send_json(&body)
            .map_err(|e| format!("Twist order submission to {url} failed: {e}"))
            .and_then(|mut response| {
                response
                    .body_mut()
                    .read_json::<TwistOrderResponse>()
                    .map_err(|e| format!("Twist order response from {url} was not valid JSON: {e}"))
            });

        match result {
            Ok(order) => Box::new(TwistJobHandle {
                status_url: format!("{}{}", self.config.base_url, self.config.status_path_template.replace("{id}", &order.id)),
                auth_header_name: self.config.auth_header_name.clone(),
                api_token: self.config.api_token.clone(),
                email_header_name: self.config.email_header_name.clone(),
                email: self.config.email.clone(),
                agent: self.agent.clone(),
            }),
            Err(e) => Box::new(ImmediateJobHandle::new(Err(e))),
        }
    }
}

struct TwistJobHandle {
    status_url: String,
    auth_header_name: String,
    api_token: String,
    email_header_name: String,
    email: String,
    agent: ureq::Agent,
}

impl JobHandle for TwistJobHandle {
    fn status(&self) -> JobStatus {
        let result = self
            .agent
            .get(&self.status_url)
            .header(&self.auth_header_name, format!("Token {}", self.api_token))
            .header(&self.email_header_name, &self.email)
            .call()
            .map_err(|e| format!("Twist order status poll of {} failed: {e}", self.status_url))
            .and_then(|mut response| {
                response
                    .body_mut()
                    .read_json::<TwistOrderStatusResponse>()
                    .map_err(|e| format!("Twist order status response from {} was not valid JSON: {e}", self.status_url))
            });

        match result {
            Ok(status) => classify_order_status(&status.status),
            Err(e) => JobStatus::Failed(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::synthesis_request;

    fn test_config(base_url: String) -> TwistConfig {
        TwistConfig::new(base_url, "/orders", "/orders/{id}", "lab@example.com", "test-token")
    }

    /// A minimal local HTTP server standing in for TAPI, so the real
    /// request-building/response-parsing code runs end-to-end without
    /// touching Twist's live API (which this crate has no credentials
    /// for).
    fn spawn_fake_tapi() -> (tiny_http::Server, String) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr();
        (server, format!("http://{addr}"))
    }

    fn handle_one_order_then_one_status(server: tiny_http::Server, order_id: &str, status: &str, captured_headers: std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>) {
        std::thread::spawn({
            let order_id = order_id.to_string();
            let status = status.to_string();
            move || {
                for _ in 0..2 {
                    let mut request = server.recv().unwrap();
                    for header in request.headers() {
                        captured_headers.lock().unwrap().push((header.field.to_string(), header.value.to_string()));
                    }
                    let mut body = String::new();
                    let _ = request.as_reader().read_to_string(&mut body);
                    let response_body = if body.contains("constructs") {
                        format!("{{\"id\":\"{order_id}\"}}")
                    } else {
                        format!("{{\"status\":\"{status}\"}}")
                    };
                    let response = tiny_http::Response::from_string(response_body)
                        .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap());
                    request.respond(response).unwrap();
                }
            }
        });
    }

    #[test]
    fn submit_and_poll_status_round_trip_against_a_fake_server() {
        let (server, base_url) = spawn_fake_tapi();
        let headers = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        handle_one_order_then_one_status(server, "order-123", "in_progress", headers.clone());

        let provider = TwistProvider::new(test_config(base_url));
        let handle = provider.submit(&[synthesis_request("gene_a.fasta")]);
        let status = handle.status();

        assert_eq!(status, JobStatus::Running);
        let seen = headers.lock().unwrap();
        assert!(seen.iter().any(|(name, value)| name.eq_ignore_ascii_case("Authorization") && value.contains("test-token")));
        assert!(seen.iter().any(|(name, value)| name.eq_ignore_ascii_case("X-Twist-Email") && value == "lab@example.com"));
    }

    #[test]
    fn completed_status_is_classified_as_complete() {
        let (server, base_url) = spawn_fake_tapi();
        handle_one_order_then_one_status(server, "order-456", "shipped", std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));

        let provider = TwistProvider::new(test_config(base_url));
        let handle = provider.submit(&[synthesis_request("gene_b.fasta")]);
        assert!(matches!(handle.status(), JobStatus::Complete(_)));
    }

    #[test]
    fn cancelled_status_is_classified_as_failed() {
        let (server, base_url) = spawn_fake_tapi();
        handle_one_order_then_one_status(server, "order-789", "cancelled", std::sync::Arc::new(std::sync::Mutex::new(Vec::new())));

        let provider = TwistProvider::new(test_config(base_url));
        let handle = provider.submit(&[synthesis_request("gene_c.fasta")]);
        assert!(matches!(handle.status(), JobStatus::Failed(_)));
    }

    #[test]
    fn unreachable_base_url_fails_submit_cleanly() {
        let provider = TwistProvider::new(test_config("http://127.0.0.1:1".to_string()));
        let handle = provider.submit(&[synthesis_request("gene_d.fasta")]);
        assert!(matches!(handle.status(), JobStatus::Failed(_)));
    }

    #[test]
    fn provider_name_is_twist() {
        let provider = TwistProvider::new(test_config("http://127.0.0.1:0".to_string()));
        assert_eq!(provider.name(), "twist");
    }
}
