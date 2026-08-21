use nucle_hardware::nanopore::pb::minknow_api::{manager, protocol};
use nucle_hardware::{JobStatus, NanoporeAuth, NanoporeConfig, NanoporeProvider, Provider};
use std::convert::Infallible;
use std::net::TcpListener as StdTcpListener;
use std::pin::Pin;
use std::task::{Context, Poll};
use tonic::codegen::{http, tokio_stream, Body, BoxFuture, Service, StdError};
use tonic::server::{NamedService, ServerStreamingService, UnaryService};
use tonic::transport::{Identity, Server, ServerTlsConfig};
use tonic::{Request, Response, Status};

#[derive(Clone)]
struct ManagerMock {
    position_port: u16,
}

impl NamedService for ManagerMock {
    const NAME: &'static str = "minknow_api.manager.ManagerService";
}

impl<B> Service<http::Request<B>> for ManagerMock
where
    B: Body + Send + 'static,
    B::Error: Into<StdError> + Send + 'static,
{
    type Response = http::Response<tonic::body::Body>;
    type Error = Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        if request.uri().path() == "/minknow_api.manager.ManagerService/flow_cell_positions" {
            let method = FlowCellPositionsMock {
                position_port: self.position_port,
            };
            return Box::pin(async move {
                let mut grpc = tonic::server::Grpc::new(tonic_prost::ProstCodec::default());
                Ok(grpc.server_streaming(method, request).await)
            });
        }
        unimplemented_response()
    }
}

struct FlowCellPositionsMock {
    position_port: u16,
}

impl ServerStreamingService<manager::FlowCellPositionsRequest> for FlowCellPositionsMock {
    type Response = manager::FlowCellPositionsResponse;
    type ResponseStream = OneItemStream<Self::Response>;
    type Future = BoxFuture<Response<Self::ResponseStream>, Status>;

    fn call(&mut self, _request: Request<manager::FlowCellPositionsRequest>) -> Self::Future {
        let response = manager::FlowCellPositionsResponse {
            total_count: 1,
            positions: vec![manager::FlowCellPosition {
                name: "MOCK_POSITION".to_string(),
                state: manager::flow_cell_position::State::Running as i32,
                rpc_ports: Some(manager::flow_cell_position::RpcPorts {
                    secure: self.position_port as u32,
                    ..Default::default()
                }),
                ..Default::default()
            }],
        };
        Box::pin(async move { Ok(Response::new(OneItemStream(Some(Ok(response))))) })
    }
}

#[derive(Clone)]
struct ProtocolMock;

impl NamedService for ProtocolMock {
    const NAME: &'static str = "minknow_api.protocol.ProtocolService";
}

impl<B> Service<http::Request<B>> for ProtocolMock
where
    B: Body + Send + 'static,
    B::Error: Into<StdError> + Send + 'static,
{
    type Response = http::Response<tonic::body::Body>;
    type Error = Infallible;
    type Future = BoxFuture<Self::Response, Self::Error>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<B>) -> Self::Future {
        match request.uri().path() {
            "/minknow_api.protocol.ProtocolService/start_protocol" => Box::pin(async move {
                let mut grpc = tonic::server::Grpc::new(tonic_prost::ProstCodec::default());
                Ok(grpc.unary(StartProtocolMock, request).await)
            }),
            "/minknow_api.protocol.ProtocolService/get_run_info" => Box::pin(async move {
                let mut grpc = tonic::server::Grpc::new(tonic_prost::ProstCodec::default());
                Ok(grpc.unary(GetRunInfoMock, request).await)
            }),
            _ => unimplemented_response(),
        }
    }
}

struct StartProtocolMock;

impl UnaryService<protocol::StartProtocolRequest> for StartProtocolMock {
    type Response = protocol::StartProtocolResponse;
    type Future = BoxFuture<Response<Self::Response>, Status>;

    fn call(&mut self, request: Request<protocol::StartProtocolRequest>) -> Self::Future {
        Box::pin(async move {
            assert_eq!(request.into_inner().identifier, "sequencing/mock");
            Ok(Response::new(protocol::StartProtocolResponse {
                run_id: "run-123".to_string(),
            }))
        })
    }
}

struct GetRunInfoMock;

impl UnaryService<protocol::GetRunInfoRequest> for GetRunInfoMock {
    type Response = protocol::ProtocolRunInfo;
    type Future = BoxFuture<Response<Self::Response>, Status>;

    fn call(&mut self, request: Request<protocol::GetRunInfoRequest>) -> Self::Future {
        Box::pin(async move {
            assert_eq!(request.into_inner().run_id, "run-123");
            Ok(Response::new(protocol::ProtocolRunInfo {
                state: protocol::ProtocolState::ProtocolCompleted as i32,
                ..Default::default()
            }))
        })
    }
}

struct OneItemStream<T>(Option<Result<T, Status>>);

impl<T: Unpin> tokio_stream::Stream for OneItemStream<T> {
    type Item = Result<T, Status>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Ready(self.0.take())
    }
}

fn unimplemented_response() -> BoxFuture<http::Response<tonic::body::Body>, Infallible> {
    Box::pin(async move {
        let mut response = http::Response::new(tonic::body::Body::default());
        response.headers_mut().insert(
            tonic::Status::GRPC_STATUS,
            (tonic::Code::Unimplemented as i32).into(),
        );
        response.headers_mut().insert(
            http::header::CONTENT_TYPE,
            tonic::metadata::GRPC_CONTENT_TYPE,
        );
        Ok(response)
    })
}

fn spawn_server<S>(listener: StdTcpListener, service: S, cert_pem: String, key_pem: String)
where
    S: Service<
            http::Request<tonic::body::Body>,
            Response = http::Response<tonic::body::Body>,
            Error = Infallible,
        > + NamedService
        + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send + 'static,
{
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            listener.set_nonblocking(true).unwrap();
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            let incoming = tonic::transport::server::TcpIncoming::from(listener);
            Server::builder()
                .tls_config(ServerTlsConfig::new().identity(Identity::from_pem(cert_pem, key_pem)))
                .unwrap()
                .add_service(service)
                .serve_with_incoming(incoming)
                .await
                .unwrap();
        });
    });
}

#[test]
fn submit_and_resume_status_against_a_local_minknow_grpc_server() {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_pem = certified.cert.pem();
    let key_pem = certified.key_pair.serialize_pem();

    let protocol_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let protocol_port = protocol_listener.local_addr().unwrap().port();
    let manager_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let manager_port = manager_listener.local_addr().unwrap().port();
    spawn_server(
        protocol_listener,
        ProtocolMock,
        cert_pem.clone(),
        key_pem.clone(),
    );
    spawn_server(
        manager_listener,
        ManagerMock {
            position_port: protocol_port,
        },
        cert_pem.clone(),
        key_pem,
    );

    let config = || NanoporeConfig {
        host: "127.0.0.1".to_string(),
        manager_port,
        ca_certificate_pem: cert_pem.as_bytes().to_vec(),
        auth: NanoporeAuth::DeveloperApiToken("mock-token".to_string()),
        protocol_identifier: "sequencing/mock".to_string(),
    };
    let provider = NanoporeProvider::new(config()).unwrap();
    let handle = provider.submit(&[]);
    let token = handle
        .resume_token()
        .expect("accepted run must be resumable");

    let resumed_provider = NanoporeProvider::new(config()).unwrap();
    let resumed = resumed_provider.resume(&token).unwrap();
    assert!(matches!(resumed.status(), JobStatus::Complete(_)));
}
