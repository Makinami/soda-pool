#![allow(unexpected_cfgs)]
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![cfg_attr(coverage_nightly, coverage(off))]

fn main() {
    let mut config = prost_build::Config::new();
    config.protoc_arg("--experimental_allow_proto3_optional");

    tonic_build::configure()
        .out_dir("./example-protobuf.gen/src")
        .compile_protos_with_config(config, &["./proto/example.proto"], &["./proto"])
        .expect("failed to compile proto file");
}
