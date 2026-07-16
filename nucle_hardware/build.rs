//! Compiles the real, vendored MinKNOW gRPC protos (see
//! `proto/minknow_api/README.md`) into Rust types/client stubs for
//! `src/nanopore.rs`. Uses a vendored `protoc` binary rather than requiring
//! one to be preinstalled, since none of this project's CI runners
//! (ubuntu-latest/macos-latest/windows-latest) have it by default.

fn main() {
    let protoc_path = protoc_bin_vendored::protoc_bin_path().expect("failed to locate vendored protoc binary");
    std::env::set_var("PROTOC", protoc_path);

    tonic_prost_build::configure()
        .build_server(false)
        .compile_protos(
            &["proto/minknow_api/manager.proto", "proto/minknow_api/protocol.proto"],
            &["proto"],
        )
        .expect("failed to compile vendored minknow_api protos");
}
