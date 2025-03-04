fn main() {
    let mut config = prost_build::Config::new();
    config.protoc_arg("--experimental_allow_proto3_optional");

    tonic_build::configure()
        .out_dir(format!("./example-protobuf.gen/src"))
        .compile_with_config(config, &[format!("./proto/example.proto")], &["./proto"])
        .expect("failed to compile proto file");
}
