fn main() {
    println!("protoc path: {}", protobuf_src::protoc().to_string_lossy());
    std::env::set_var("PROTOC", protobuf_src::protoc());
    tonic_build::configure()
        .out_dir("src/")
        .extern_path(".google.protobuf.Timestamp", "::prost_wkt_types::Timestamp")
        .compile(&["proto/rlog-service.proto"], &["proto"])
        .unwrap();
}
