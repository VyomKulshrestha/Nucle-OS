//! A real gRPC client against Oxford Nanopore's public MinKNOW API.
//!
//! Unlike Twist/IDT/Illumina, Oxford Nanopore has no cloud REST API to
//! submit a sample to -- MinKNOW is a *local* gRPC service that controls a
//! directly-attached sequencer (see `proto/minknow_api/README.md` for the
//! vendored, real `.proto` files this module compiles against, and ONT's
//! own `AUTH.md` for the authentication model this mirrors). There is no
//! ONT hardware in this project's development environment, so this has
//! never been run against a live instrument -- but every RPC call below is
//! the real, documented `ManagerService`/`ProtocolService` wire protocol,
//! not a mock.
//!
//! A `submit()` call here starts one MinKNOW protocol run per batch (a
//! flow-cell run typically serves one physical experiment, not one request
//! each) and passes each request's target file name through as a protocol
//! argument; `JobHandle::status()` polls `ProtocolService.get_run_info`
//! and maps `ProtocolState` onto `JobStatus`.

use crate::provider::{ImmediateJobHandle, JobHandle, JobStatus, Provider};
use nucle_lang::hardware::HardwareRequest;
use std::sync::Arc;
use tonic::transport::{Certificate, Channel, ClientTlsConfig, Endpoint, Identity};
use tonic::Request;

#[allow(clippy::all)]
pub mod pb {
    pub mod minknow_api {
        pub mod manager {
            tonic::include_proto!("minknow_api.manager");
        }
        pub mod protocol {
            tonic::include_proto!("minknow_api.protocol");
        }
        pub mod device {
            tonic::include_proto!("minknow_api.device");
        }
        pub mod instance {
            tonic::include_proto!("minknow_api.instance");
        }
        pub mod protocol_settings {
            tonic::include_proto!("minknow_api.protocol_settings");
        }
        pub mod acquisition {
            tonic::include_proto!("minknow_api.acquisition");
        }
        pub mod analysis_workflows {
            tonic::include_proto!("minknow_api.analysis_workflows");
        }
        pub mod basecaller {
            tonic::include_proto!("minknow_api.basecaller");
        }
        pub mod analysis_configuration {
            tonic::include_proto!("minknow_api.analysis_configuration");
        }
        pub mod run_until {
            tonic::include_proto!("minknow_api.run_until");
        }
        pub mod read_end_reason {
            tonic::include_proto!("minknow_api.read_end_reason");
        }
    }
}

use pb::minknow_api::manager::{
    flow_cell_position::State as PositionState, manager_service_client::ManagerServiceClient,
    FlowCellPositionsRequest,
};
use pb::minknow_api::protocol::{
    protocol_service_client::ProtocolServiceClient, GetRunInfoRequest, ProtocolState,
    StartProtocolRequest,
};

/// How the client authenticates to MinKNOW. Client certificates are ONT's
/// recommended approach; a developer API token (deprecated by ONT, but far
/// simpler to configure for a one-off integration) is sent as gRPC metadata
/// under the same `local-auth` key MinKNOW's own Python client uses --
/// see `AUTH.md` in the vendored proto's source repo.
#[derive(Clone)]
pub enum NanoporeAuth {
    /// A developer API token generated from the MinKNOW UI's Host Settings.
    DeveloperApiToken(String),
    /// A client certificate chain + private key (PEM), signed by (or
    /// itself present in) the instrument's `conf/rpc-client-certs`.
    ClientCertificate { cert_chain_pem: Vec<u8>, key_pem: Vec<u8> },
}

pub struct NanoporeConfig {
    /// Host/IP of the machine running MinKNOW -- `"127.0.0.1"` for a
    /// directly-attached sequencer.
    pub host: String,
    /// The manager service's port. MinKNOW's default secure manager ports
    /// are 9502 (gRPC-Web compatible) and 9501 (client-cert-only).
    pub manager_port: u16,
    /// The (PEM-encoded) CA certificate that signed MinKNOW's self-signed
    /// server certificate -- by default found at
    /// `<data_dir>/rpc-certs/minknow/ca.crt` on the instrument itself.
    /// There's no well-known path this crate can read from a remote
    /// machine, so it must be supplied explicitly.
    pub ca_certificate_pem: Vec<u8>,
    pub auth: NanoporeAuth,
    /// The protocol identifier to start, exactly as `list_protocols()`
    /// would report it on the target instrument (e.g.
    /// `"sequencing/sequencing_MIN106_DNA:FLO-MIN106:SQK-LSK109"`). This is
    /// specific to the flow cell/kit loaded on the instrument -- there is
    /// no universal default to fall back to.
    pub protocol_identifier: String,
}

/// A real gRPC `Provider` for Oxford Nanopore's MinKNOW API. See the module
/// doc comment for what "real" does and doesn't mean here.
pub struct NanoporeProvider {
    config: NanoporeConfig,
    developer_token: Option<String>,
    runtime: Arc<tokio::runtime::Runtime>,
}

impl NanoporeProvider {
    pub fn new(config: NanoporeConfig) -> Result<Self, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| format!("failed to start a Tokio runtime for NanoporeProvider: {e}"))?;
        let developer_token = match &config.auth {
            NanoporeAuth::DeveloperApiToken(token) => Some(token.clone()),
            NanoporeAuth::ClientCertificate { .. } => None,
        };
        Ok(Self { config, developer_token, runtime: Arc::new(runtime) })
    }

    fn tls_config(&self) -> ClientTlsConfig {
        let mut tls = ClientTlsConfig::new()
            .ca_certificate(Certificate::from_pem(&self.config.ca_certificate_pem))
            // MinKNOW's self-signed server certificate's CN is "localhost"
            // regardless of the real hostname -- this mirrors
            // minknow_api's own `grpc.ssl_target_name_override` behavior,
            // not a shortcut taken here.
            .domain_name("localhost");
        if let NanoporeAuth::ClientCertificate { cert_chain_pem, key_pem } = &self.config.auth {
            tls = tls.identity(Identity::from_pem(cert_chain_pem, key_pem));
        }
        tls
    }

    fn connect(&self, host: &str, port: u16) -> Result<Channel, String> {
        let endpoint = Endpoint::from_shared(format!("https://{host}:{port}"))
            .map_err(|e| format!("invalid MinKNOW endpoint {host}:{port}: {e}"))?
            .tls_config(self.tls_config())
            .map_err(|e| format!("invalid MinKNOW TLS config: {e}"))?;
        self.runtime
            .block_on(endpoint.connect())
            .map_err(|e| format!("failed to connect to MinKNOW at {host}:{port}: {e}"))
    }

    fn with_auth<T>(&self, mut request: Request<T>) -> Request<T> {
        if let Some(token) = &self.developer_token {
            if let Ok(value) = token.parse() {
                request.metadata_mut().insert("local-auth", value);
            }
        }
        request
    }

    /// Finds the first flow cell position in `STATE_RUNNING`, returning its
    /// name and the secure gRPC port its own `ProtocolService` is exposed on
    /// (`ManagerService.flow_cell_positions` -- a real RPC, per
    /// `manager.proto`, that "can be called without providing any
    /// authentication tokens").
    fn discover_running_position(&self) -> Result<(String, u16), String> {
        let channel = self.connect(&self.config.host, self.config.manager_port)?;
        let mut client = ManagerServiceClient::new(channel);
        let request = self.with_auth(Request::new(FlowCellPositionsRequest {}));
        let mut stream = self
            .runtime
            .block_on(client.flow_cell_positions(request))
            .map_err(|e| format!("flow_cell_positions failed: {e}"))?
            .into_inner();
        loop {
            let next = self
                .runtime
                .block_on(stream.message())
                .map_err(|e| format!("flow_cell_positions stream error: {e}"))?;
            let Some(response) = next else {
                return Err(
                    "MinKNOW reported no flow cell position in STATE_RUNNING".to_string()
                );
            };
            for position in response.positions {
                if position.state == PositionState::Running as i32 {
                    if let Some(ports) = position.rpc_ports {
                        return Ok((position.name, ports.secure as u16));
                    }
                }
            }
        }
    }
}

impl Provider for NanoporeProvider {
    fn name(&self) -> &str {
        "nanopore"
    }

    fn submit(&self, batch: &[HardwareRequest]) -> Box<dyn JobHandle> {
        let args: Vec<String> = batch.iter().map(|request| request.target.clone()).collect();
        let outcome = (|| -> Result<(Channel, String), String> {
            let (_position_name, position_port) = self.discover_running_position()?;
            let channel = self.connect(&self.config.host, position_port)?;
            let mut client = ProtocolServiceClient::new(channel.clone());
            let request = self.with_auth(Request::new(StartProtocolRequest {
                identifier: self.config.protocol_identifier.clone(),
                args,
                ..Default::default()
            }));
            let response = self
                .runtime
                .block_on(client.start_protocol(request))
                .map_err(|e| format!("start_protocol failed: {e}"))?
                .into_inner();
            Ok((channel, response.run_id))
        })();

        match outcome {
            Ok((channel, run_id)) => Box::new(NanoporeJobHandle {
                channel,
                run_id,
                developer_token: self.developer_token.clone(),
                runtime: Arc::clone(&self.runtime),
            }),
            Err(e) => Box::new(ImmediateJobHandle::new(Err(e))),
        }
    }
}

struct NanoporeJobHandle {
    channel: Channel,
    run_id: String,
    developer_token: Option<String>,
    runtime: Arc<tokio::runtime::Runtime>,
}

impl NanoporeJobHandle {
    fn with_auth<T>(&self, mut request: Request<T>) -> Request<T> {
        if let Some(token) = &self.developer_token {
            if let Ok(value) = token.parse() {
                request.metadata_mut().insert("local-auth", value);
            }
        }
        request
    }
}

impl JobHandle for NanoporeJobHandle {
    fn status(&self) -> JobStatus {
        let mut client = ProtocolServiceClient::new(self.channel.clone());
        let request =
            self.with_auth(Request::new(GetRunInfoRequest { run_id: self.run_id.clone() }));
        match self.runtime.block_on(client.get_run_info(request)) {
            Ok(response) => map_protocol_state(response.into_inner().state, &self.run_id),
            Err(status) => {
                JobStatus::Failed(format!("get_run_info failed for run {}: {status}", self.run_id))
            }
        }
    }
}

fn map_protocol_state(state: i32, run_id: &str) -> JobStatus {
    match ProtocolState::try_from(state) {
        Ok(ProtocolState::ProtocolCompleted) => {
            JobStatus::Complete(format!("MinKNOW protocol run {run_id} completed"))
        }
        Ok(ProtocolState::ProtocolRunning)
        | Ok(ProtocolState::ProtocolWaitingForTemperature)
        | Ok(ProtocolState::ProtocolWaitingForAcquisition)
        | Ok(ProtocolState::ProtocolWaitingForResource) => JobStatus::Running,
        Ok(other) => JobStatus::Failed(format!(
            "MinKNOW protocol run {run_id} ended in state {other:?}"
        )),
        Err(_) => JobStatus::Failed(format!(
            "MinKNOW protocol run {run_id} reported an unrecognized protocol state {state}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ProtocolState`'s exact variant names/discriminants come from the
    /// vendored, real `protocol.proto` -- this pins `map_protocol_state`'s
    /// interpretation of them so a future proto re-vendor that renumbers
    /// the enum can't silently flip a completion into a failure or vice
    /// versa.
    #[test]
    fn map_protocol_state_treats_completed_as_success() {
        let status = map_protocol_state(ProtocolState::ProtocolCompleted as i32, "run-1");
        assert!(matches!(status, JobStatus::Complete(_)));
    }

    #[test]
    fn map_protocol_state_treats_in_progress_states_as_running() {
        for state in [
            ProtocolState::ProtocolRunning,
            ProtocolState::ProtocolWaitingForTemperature,
            ProtocolState::ProtocolWaitingForAcquisition,
            ProtocolState::ProtocolWaitingForResource,
        ] {
            assert_eq!(map_protocol_state(state as i32, "run-1"), JobStatus::Running);
        }
    }

    #[test]
    fn map_protocol_state_treats_error_and_user_stop_as_failed() {
        for state in [
            ProtocolState::ProtocolStoppedByUser,
            ProtocolState::ProtocolFinishedWithError,
            ProtocolState::ProtocolFinishedWithDeviceError,
        ] {
            assert!(matches!(map_protocol_state(state as i32, "run-1"), JobStatus::Failed(_)));
        }
    }

    #[test]
    fn map_protocol_state_fails_closed_on_an_unrecognized_discriminant() {
        // Not a valid ProtocolState value in the vendored proto -- must not
        // be silently treated as success or "still running".
        assert!(matches!(map_protocol_state(999, "run-1"), JobStatus::Failed(_)));
    }

    #[test]
    fn nanopore_provider_name_is_nanopore() {
        let config = NanoporeConfig {
            host: "127.0.0.1".to_string(),
            manager_port: 9502,
            ca_certificate_pem: Vec::new(),
            auth: NanoporeAuth::DeveloperApiToken("token".to_string()),
            protocol_identifier: "sequencing/dummy".to_string(),
        };
        let provider = NanoporeProvider::new(config).unwrap();
        assert_eq!(provider.name(), "nanopore");
    }

    #[test]
    fn submit_without_a_reachable_minknow_instance_fails_rather_than_hangs() {
        // No MinKNOW manager is running in CI/dev -- this must fail
        // cleanly (connection refused) rather than block forever, since
        // `submit()` calls `block_on` synchronously.
        let config = NanoporeConfig {
            host: "127.0.0.1".to_string(),
            manager_port: 9502,
            ca_certificate_pem: Vec::new(),
            auth: NanoporeAuth::DeveloperApiToken("token".to_string()),
            protocol_identifier: "sequencing/dummy".to_string(),
        };
        let provider = NanoporeProvider::new(config).unwrap();
        let result = provider.execute_batch(&[]);
        assert!(result.is_err());
    }
}
