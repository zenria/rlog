# rlog - remote logging tools

Status: Experimental

rlog collects logs locally and ship them to a remote logging destination. Collected logs are written to a [Quickwit](https://quickwit.io/) instance.

- collect logs locally using syslog UDP protocol or GELF (graylog) protocol
- send logs to a remote log collector (mTLS gRPC)
- write logs to quickwit

It provides a secure external interface with a minimal attack surface.

## Open Telemetry?

Why not use Open Telemetry ecosystem and try to do hacky stuff instead?

This software aims to be really easy to deploy on existing stacks (syslog&gelf). It
is also an experiment used to validate some concepts. (especially quickwit)

While the indexing schema seems bad ;

## rlog-shipper

rlog-shipper collects logs locally and sends them to a remote log collector.

Logs are collected locally using:

- GELF protocol
- syslog UDP protocol

And sent to the log collector using gRPC secured with mTLS. The protocol is described
in [rlog-service.proto](rlog-grpc/proto/rlog-service.proto)

## rlog-collector

=> **TODO**

- export to quickvit using OTEL grpc protocol?
- export to quickvit using custom schema?

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
