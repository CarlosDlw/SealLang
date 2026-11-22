//! Generates the REAPI client and server bindings.

fn main() {
    let client = std::env::var_os("CARGO_FEATURE_CLIENT").is_some();
    let server = std::env::var_os("CARGO_FEATURE_SERVER").is_some();
    println!("cargo:rerun-if-changed=proto");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_CLIENT");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_SERVER");
    let mut config = prost_build::Config::new();
    config.extern_path(".google.longrunning", "crate::proto::google::longrunning");
    config.extern_path(".google.rpc", "crate::proto::google::rpc");
    tonic_build::configure()
        .build_client(client)
        .build_server(server)
        .compile_protos_with_config(
            config,
            &[
                "proto/build/bazel/remote/execution/v2/remote_execution.proto",
                "proto/google/bytestream/bytestream.proto",
            ],
            &["proto"],
        )
        .expect("the REAPI protocol definitions compile");
}
