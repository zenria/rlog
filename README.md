# rlog - remote logging tools

Status: Experimental

rlog collects logs locally and ship them to a remote logging destination. Collected logs are written to a [Quickwit](https://quickwit.io/) instance.

- collect logs locally using syslog UDP protocol or GELF (graylog) protocol
- send logs to a remote log collector (mTLS gRPC)
- write logs to quickwit

It provides a secure external interface with a minimal attack surface.

## rlog-shipper

rlog-shipper collects logs locally and sends them to a remote log collector.

Logs are collected locally using:

- GELF protocol
- syslog UDP protocol

And sent to the log collector using gRPC secured with mTLS. The protocol is described
in [rlog-service.proto](rlog-grpc/proto/rlog-service.proto)

## rlog-collector

_TODO_
