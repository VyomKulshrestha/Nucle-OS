//! A real HTTP client against IDT's (Integrated DNA Technologies) SciTools
//! Plus API for oligo/gene order submission.
//!
//! Confirmed from IDT's public developer documentation
//! (`idtdna.com/pages/tools/apidoc`): the OAuth2 token endpoint is
//! `https://www.idtdna.com/Identityserver/connect/token`, and it's reached
//! via HTTP Basic auth (`client_id:client_secret`, base64-encoded) with a
//! `grant_type`/`username`/`password` form body -- IDT's flow needs an
//! actual IDT account username/password in addition to the client
//! credentials, not just a plain OAuth2 client-credentials grant. The
//! resulting bearer token then authorizes the rest of the API.
//!
//! What is *not* independently confirmed here -- because SciTools Plus's
//! full endpoint reference sits behind account-gated Swagger/Postman
//! documentation this crate's author has no IDT account to view -- is the
//! exact order-submission endpoint path and JSON schema. `IdtConfig`
//! requires those explicitly rather than guessing at defaults.
//!
//! Untested against a live IDT account -- there are no SciTools Plus
//! credentials available in this project's development environment -- but
//! every request below, including the token exchange, is a genuine HTTP
//! call, not a mock.

use crate::provider::{ImmediateJobHandle, JobHandle, JobStatus, Provider};
use base64::Engine;
use nucle_lang::hardware::HardwareRequest;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct IdtConfig {
    pub client_id: String,
    pub client_secret: String,
    /// IDT's OAuth2 flow authenticates an actual account, not just the API
    /// client -- these are IDT account credentials, not API keys.
    pub username: String,
    pub password: String,
    /// Real, confirmed token endpoint: defaults to
    /// `https://www.idtdna.com/Identityserver/connect/token`.
    pub token_url: String,
    /// SciTools Plus API base URL for order-related calls -- required
    /// rather than defaulted since the exact production host isn't public
    /// without an account.
    pub api_base_url: String,
    /// Path (relative to `api_base_url`) that creates a new order --
    /// verify against your account's Swagger/Postman reference.
    pub order_path: String,
    /// Path template for polling an order's status, with `{id}` replaced
    /// by the id the order endpoint returned.
    pub status_path_template: String,
}

impl IdtConfig {
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>, username: impl Into<String>, password: impl Into<String>, api_base_url: impl Into<String>, order_path: impl Into<String>, status_path_template: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            username: username.into(),
            password: password.into(),
            token_url: "https://www.idtdna.com/Identityserver/connect/token".to_string(),
            api_base_url: api_base_url.into(),
            order_path: order_path.into(),
            status_path_template: status_path_template.into(),
        }
    }
}

#[derive(Deserialize)]
struct IdtTokenResponse {
    access_token: String,
}

#[derive(Serialize)]
struct IdtOrderRequest {
    name: String,
    items: Vec<IdtOrderItem>,
}

#[derive(Serialize)]
struct IdtOrderItem {
    name: String,
}

#[derive(Deserialize)]
struct IdtOrderResponse {
    id: String,
}

#[derive(Deserialize)]
struct IdtOrderStatusResponse {
    status: String,
}

/// A real HTTP `Provider` for IDT's SciTools Plus API. See the module doc
/// comment for what "real" does and doesn't mean here.
pub struct IdtProvider {
    config: IdtConfig,
    agent: ureq::Agent,
}

impl IdtProvider {
    pub fn new(config: IdtConfig) -> Self {
        Self { config, agent: crate::http_client::new_agent() }
    }
}

/// Exchanges account credentials for a bearer token via the real,
/// confirmed IDT token endpoint. Fetched fresh on every call rather than
/// cached/refreshed -- correct but not efficient for high call volume; a
/// production integration would want to cache this until `expires_in`.
/// A free function (rather than a `Provider` method) so both
/// `IdtProvider::submit` and `IdtJobHandle::status` can call it without
/// either owning the other.
fn fetch_access_token(agent: &ureq::Agent, config: &IdtConfig) -> Result<String, String> {
    let basic = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", config.client_id, config.client_secret));
    let form = format!(
        "grant_type=password&username={}&password={}",
        urlencode(&config.username),
        urlencode(&config.password),
    );
    agent
        .post(&config.token_url)
        .header("Authorization", format!("Basic {basic}"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .send(form.as_str())
        .map_err(|e| format!("IDT token exchange with {} failed: {e}", config.token_url))
        .and_then(|mut response| {
            response
                .body_mut()
                .read_json::<IdtTokenResponse>()
                .map_err(|e| format!("IDT token response was not valid JSON: {e}"))
        })
        .map(|token| token.access_token)
}

fn urlencode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Best-effort classification of SciTools Plus's (unconfirmed)
/// order-status vocabulary -- see the module doc comment. Kept as a
/// standalone function so it's the one place to fix if the real vendor's
/// wording differs.
fn classify_order_status(status: &str) -> JobStatus {
    let lower = status.to_ascii_lowercase();
    if ["complete", "completed", "shipped", "delivered", "fulfilled"]
        .iter()
        .any(|s| lower.contains(s))
    {
        JobStatus::Complete(format!("IDT order status: {status}"))
    } else if ["cancelled", "canceled", "failed", "error", "rejected"]
        .iter()
        .any(|s| lower.contains(s))
    {
        JobStatus::Failed(format!("IDT order status: {status}"))
    } else {
        JobStatus::Running
    }
}

impl Provider for IdtProvider {
    fn name(&self) -> &str {
        "idt"
    }

    fn submit(&self, batch: &[HardwareRequest]) -> Box<dyn JobHandle> {
        let outcome = (|| -> Result<String, String> {
            let access_token = fetch_access_token(&self.agent, &self.config)?;
            let items = batch.iter().map(|request| IdtOrderItem { name: request.target.clone() }).collect();
            let body = IdtOrderRequest { name: format!("NucleOS batch ({} items)", batch.len()), items };

            let url = format!("{}{}", self.config.api_base_url, self.config.order_path);
            let order = self
                .agent
                .post(&url)
                .header("Authorization", format!("Bearer {access_token}"))
                .send_json(&body)
                .map_err(|e| format!("IDT order submission to {url} failed: {e}"))?
                .body_mut()
                .read_json::<IdtOrderResponse>()
                .map_err(|e| format!("IDT order response from {url} was not valid JSON: {e}"))?;
            Ok(order.id)
        })();

        match outcome {
            Ok(order_id) => Box::new(IdtJobHandle {
                status_url: format!("{}{}", self.config.api_base_url, self.config.status_path_template.replace("{id}", &order_id)),
                config: self.config.clone(),
                agent: self.agent.clone(),
            }),
            Err(e) => Box::new(ImmediateJobHandle::new(Err(e))),
        }
    }
}

struct IdtJobHandle {
    status_url: String,
    config: IdtConfig,
    agent: ureq::Agent,
}

impl JobHandle for IdtJobHandle {
    fn status(&self) -> JobStatus {
        let result = fetch_access_token(&self.agent, &self.config)
            .and_then(|access_token| {
                self.agent
                    .get(&self.status_url)
                    .header("Authorization", format!("Bearer {access_token}"))
                    .call()
                    .map_err(|e| format!("IDT order status poll of {} failed: {e}", self.status_url))
            })
            .and_then(|mut response| {
                response
                    .body_mut()
                    .read_json::<IdtOrderStatusResponse>()
                    .map_err(|e| format!("IDT order status response from {} was not valid JSON: {e}", self.status_url))
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
    use std::sync::{Arc, Mutex};

    fn test_config(api_base_url: String, token_url: String) -> IdtConfig {
        let mut config = IdtConfig::new("client-id", "client-secret", "user@example.com", "hunter2", api_base_url, "/orders", "/orders/{id}");
        config.token_url = token_url;
        config
    }

    /// A minimal local HTTP server standing in for both the IDT token
    /// endpoint and the order API, so the real token-exchange and
    /// order-submission code runs end-to-end without touching IDT's live
    /// services (which this crate has no credentials for).
    fn spawn_fake_idt(order_id: &'static str, status: &'static str) -> (String, Arc<Mutex<Vec<String>>>) {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr();
        let paths_seen = Arc::new(Mutex::new(Vec::new()));
        let paths_seen_clone = paths_seen.clone();

        std::thread::spawn(move || {
            // 4 requests: submit() does token+order, status() does its own
            // fresh token+status poll (fetch_access_token isn't cached).
            for _ in 0..4 {
                let mut request = server.recv().unwrap();
                paths_seen_clone.lock().unwrap().push(request.url().to_string());
                let mut body = String::new();
                let _ = request.as_reader().read_to_string(&mut body);

                let response_body = if request.url().contains("token") {
                    "{\"access_token\":\"fake-bearer-token\"}".to_string()
                } else if body.contains("items") {
                    format!("{{\"id\":\"{order_id}\"}}")
                } else {
                    format!("{{\"status\":\"{status}\"}}")
                };
                let response = tiny_http::Response::from_string(response_body)
                    .with_header("Content-Type: application/json".parse::<tiny_http::Header>().unwrap());
                request.respond(response).unwrap();
            }
        });

        (format!("http://{addr}"), paths_seen)
    }

    #[test]
    fn submit_exchanges_a_token_then_creates_an_order_then_polls_status() {
        let (base_url, paths_seen) = spawn_fake_idt("order-abc", "in_progress");
        let provider = IdtProvider::new(test_config(base_url.clone(), format!("{base_url}/Identityserver/connect/token")));

        let handle = provider.submit(&[synthesis_request("oligo_a.fasta")]);
        let status = handle.status();

        assert_eq!(status, JobStatus::Running);
        let seen = paths_seen.lock().unwrap();
        assert!(seen.iter().any(|p| p.contains("token")), "expected a token exchange call, saw: {seen:?}");
        assert!(seen.iter().any(|p| p.contains("orders")), "expected an order call, saw: {seen:?}");
    }

    #[test]
    fn completed_status_is_classified_as_complete() {
        let (base_url, _) = spawn_fake_idt("order-def", "shipped");
        let provider = IdtProvider::new(test_config(base_url.clone(), format!("{base_url}/Identityserver/connect/token")));
        let handle = provider.submit(&[synthesis_request("oligo_b.fasta")]);
        assert!(matches!(handle.status(), JobStatus::Complete(_)));
    }

    #[test]
    fn unreachable_token_endpoint_fails_submit_cleanly() {
        let provider = IdtProvider::new(test_config("http://127.0.0.1:1".to_string(), "http://127.0.0.1:1/token".to_string()));
        let handle = provider.submit(&[synthesis_request("oligo_c.fasta")]);
        assert!(matches!(handle.status(), JobStatus::Failed(_)));
    }

    #[test]
    fn provider_name_is_idt() {
        let provider = IdtProvider::new(test_config("http://127.0.0.1:0".to_string(), "http://127.0.0.1:0/token".to_string()));
        assert_eq!(provider.name(), "idt");
    }
}
