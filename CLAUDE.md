# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

rlog is an experimental remote logging pipeline: a **shipper** runs on each host, collects logs locally and forwards them over mTLS gRPC to a central **collector**, which batches them and ingests them into [Quickwit](https://quickwit.io/).

The README is partly outdated; trust the code. Known gaps in the README:
- The shipper also tails log files (`files_in` config, regex-based parsing), not only syslog/GELF.
- GELF input is **TCP only** (null-byte framed JSON), no UDP.
- The collector already has a (hot-reloaded) config file, and integration tests already exist.
- The client certificate command is `rlog-helper cert generate-client <name>` (the README wrongly says `generate-server client`).

## Coding rules

- Idiomatic Rust.
- Code, comments and commit messages in English.
- Avoid abbreviations in identifiers; prefer full names (`configuration_store`, not `cfg_st`). A plain `i` for a basic loop counter is acceptable.

## Commands

```shell
cargo build --workspace
cargo test --workspace                       # unit + integration tests
cargo test -p rlog-shipper                   # one crate
cargo test -p integration --test end_to_end  # one integration test file
cargo test -p rlog-common -- config::dir     # filter by test path
cargo clippy --workspace --all-targets
cargo fmt --all
```

Running the binaries locally: `rlog-shipper` and `rlog-collector` load a `.env` file from the current directory (via `dotenv`), so run them from their crate directory (`cd rlog-shipper && cargo run`). Every CLI flag can also be set through the matching upper-case env var (clap `env`). Certificates are generated with `cargo run -p rlog-helper -- cert generate-ca|generate-server|generate-client ...` into `./ca` (gitignored). `rlog-helper print-quickwit-schema` prints the Quickwit index config (`rlog-helper/src/schema.yaml`).

Use `RUST_LOG=debug` (tracing `EnvFilter`, default level INFO) to see per-message debug output.

## Workspace layout

- `rlog-grpc` — protocol definition (`proto/rlog-service.proto`). `build.rs` uses `tonic-build` with a vendored `protoc` (`protobuf-src`, compiled from source: the first build is slow) and writes `src/rlog_service_protocol.rs`, which is **generated and gitignored** — edit the `.proto`, never the generated file. Re-exports `tonic`, `prost`, `prost_wkt_types`: dependent crates must use these re-exports rather than depending on tonic/prost directly. Also holds the syslog → OpenTelemetry severity mapping (`OTELSeverity`).
- `rlog-common` — shared config loading (single YAML file, or a directory of YAML fragments merged via `Extend`), both polled every 5s for hot reload; `init_logging`; `format_error` (renders an `anyhow` error chain).
- `rlog-shipper` — lib + binary, the per-host agent.
- `rlog-collector` — lib + binary, the central server.
- `rlog-helper` — standalone CLI (certificate generation with `rcgen`, Quickwit schema).
- `integration` — end-to-end tests that start a real collector, shippers and a mock Quickwit (`integration/src/quickwit_mock.rs`) in-process on random free ports (`test_utils::BindAddresses`), with plaintext gRPC (no TLS).

## Architecture

### Data flow

```
syslog UDP ─┐                                                          
GELF TCP ───┼─> per-input bounded channel ─> forward_loop ─> grpc_out channel ─> gRPC client
files_in ───┘   (input-specific type)        (TryFrom<T> for LogLine)              │ mTLS
                                                                                    v
quickwit HTTP ingest <─ index loop <─ batch channel <─ batch collector <─ gRPC LogCollector service
                                                                          (TryFrom<LogLine> for IndexLogEntry)
```

- Each shipper input (`syslog_server.rs`, `gelf_server.rs`, `log_file.rs`) produces its own type (`SyslogLog`, `GelfLog`, `GenericLog`) into an `async_channel`. The generic `forward_loop` converts any of them to the protobuf `LogLine` via a `TryFrom` impl living next to the input, then pushes to the single gRPC output channel. Adding an input = new type + `TryFrom<NewType> for LogLine` + spawn a `forward_loop` in `ShipperServer::start_shipper_server`.
- `LogLine.line` is a `oneof` (gelf / syslog / generic_log). The collector's `index.rs` maps each variant into the flat `IndexLogEntry` that is serialized as NDJSON for Quickwit (`api/v1/{index_id}/ingest`). Unknown/extra fields travel as a JSON string (`extra`) and end up flattened into `free_fields`.
- Backpressure: inputs use `try_send` and **drop** messages (incrementing error counters) when their buffer is full; `grpc_out` retries a log line forever on `Unavailable` (collector sends `Unavailable` during shutdown or when its channel is closed), drops it on `InvalidArgument`/`OutOfRange`.
- Collector `index.rs` retries batches on HTTP errors, waits 5s on 429, and on a "payload too large" 400 splits the batch in half recursively (`Batch` enum), discarding a single entry that still fails.

### Global state

- Configuration is a process-global `lazy_static! CONFIG: ArcSwap<Config>` in each of `rlog-shipper/src/config.rs` and `rlog-collector/src/config.rs`. Code reads it with `CONFIG.load()` or projects a field with `CONFIG.map(...)` (arc-swap `Access`) so hot-reloaded values are picked up. Channel/buffer sizes are only read at startup; other values (exclusion filters, batch size/interval, file parse config) are live.
- Metrics are also process-global: shipper uses `AtomicU64` statics (`rlog-shipper/src/metrics.rs`) and reports them to the collector every 30s through the `ReportMetrics` RPC; the collector re-exposes them as Prometheus metrics labelled by shipper hostname.
- Consequence for tests: several shippers in one process (e.g. `integration/tests/many_shippers.rs`) share the same `CONFIG` and counters, and tests that `CONFIG.store(...)` affect every other test in the same binary.

### Lifecycle

Both servers expose `start_*_server(config) -> Self` and `async fn shutdown(self)`. Shutdown is driven by a root `tokio_util::sync::CancellationToken` with child tokens per task; shutdown drains queues (the collector's batch task closes its input, flushes a final batch, then the index loop exits when its channel closes). Binaries trigger shutdown on SIGINT/SIGTERM.

### Collector HTTP status server

`http_status_server.rs` (axum, default `0.0.0.0:21040`): `/health`, `/version`, `/metrics` (Prometheus), `/connected-shippers` (hosts seen via metric reports in the last 90s), `/quickwit/metrics` (proxied).
