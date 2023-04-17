# rlog - remote logging tools

Status: Highly Experimental

rlog collects logs locally and ship them to a remote logging destination. Collected logs are then written to a [Quickwit](https://quickwit.io/) instance.

- collect logs locally using syslog UDP protocol or GELF (graylog) protocol
- send logs to a remote log collector (mTLS gRPC)
- write logs to quickwit

TODO:

- config on collector
- resizeable buffers
- integration tests

## Open Telemetry?

Why not using Open Telemetry ecosystem and stop trying to do hacky stuff instead?

This software aims to be really easy to deploy on existing stacks (syslog&gelf). It
is also an experiment used to validate some concepts. (especially quickwit)

This software relies on production ready Rust libraries (Tonic/Hyper/Rustls) and offers a minimal attack surface
(gRPC endpoint is secured using mTLS).

## rlog-shipper

rlog-shipper collects logs locally and sends them to a remote log collector.

Logs are collected locally using:

- GELF protocol
- syslog UDP protocol

And sent to the log collector using gRPC secured with mTLS. The protocol is described
in [rlog-service.proto](rlog-grpc/proto/rlog-service.proto)

## rlog-collector

- implements the gRPC server described in [rlog-service.proto](rlog-grpc/proto/rlog-service.proto)
- all logs are sent to quickwit
- metrics of all shippers are collected and exposed though a prometheus `/metrics` HTTP endpoint

## rlog-helper

### mTLS certificates generation

```shell
# generate a self signed certificate in ./ca directory needed to sign server&client certificates
rlog-helper cert generate-ca "My certificate authority"
# generate a server certificate signed by the ca, output is written in ./ca directory
rlog-helper cert generate-server localhost
# generate a client certificate signed by the ca, output is written in ./ca directory
rlog-helper cert generate-server client

```

## License

Licensed under either of

- Apache License, Version 2.0
   ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license
   ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

## Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
