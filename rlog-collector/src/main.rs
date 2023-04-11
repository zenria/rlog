use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use rlog_common::utils::{init_logging, read_file};
use rlog_grpc::{
    rlog_service_protocol::log_collector_server::LogCollectorServer,
    tonic::transport::{Certificate, Identity, Server, ServerTlsConfig},
};

use crate::{index::IndexLogCollectorServer, metrics::launch_async_process_collector};

mod http_status_server;
mod index;
mod metrics;

/// Collects logs locally and ship them to a remote destination
#[derive(Debug, Parser)]
struct Opts {
    /// trusted CA certficate used for mTLS connection
    #[arg(long, env)]
    tls_ca_certificate: String,
    /// private key used for mTLS connection
    #[arg(long, env)]
    tls_private_key: String,
    /// certificate, signed by the CA corresponding to the private key
    #[arg(long, env)]
    tls_certificate: String,

    #[arg(long, env)]
    grpc_bind_address: String,

    #[arg(long, env, default_value = "http://127.0.0.1:7280")]
    quickwit_rest_url: String,

    #[arg(long, env, default_value = "rlog")]
    quickwit_index_id: String,

    /// HTTP status server (/health, /metrics)
    #[arg(long, env, default_value = "0.0.0.0:21040")]
    http_status_bind_address: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(e) = dotenv::dotenv() {
        eprintln!("WARN: unable to setup dotenv (.env files): {e}");
    };
    let opts = Opts::parse();

    init_logging();

    launch_async_process_collector(Duration::from_millis(500));

    let mut server = Server::builder()
        // always setup tcp keepalive
        .tcp_keepalive(Some(Duration::from_secs(25)))
        // tls config
        .tls_config(
            ServerTlsConfig::new()
                .identity(Identity::from_pem(
                    read_file(&opts.tls_certificate).context("Cannot open certificate")?,
                    read_file(&opts.tls_private_key).context("Cannot open private key")?,
                ))
                .client_ca_root(Certificate::from_pem(
                    read_file(&opts.tls_ca_certificate).context("Cannot open ca certificate")?,
                )),
        )
        .context("Invalid TLS configuration")?;

    let addr = opts
        .grpc_bind_address
        .parse()
        .context("Invalid grpc bind address")?;

    tracing::info!("Starting rlog-collector gRPC server at {addr}");

    http_status_server::launch_server(&opts.http_status_bind_address)?;

    server
        .add_service(LogCollectorServer::new(IndexLogCollectorServer::new(
            &opts.quickwit_rest_url,
            &opts.quickwit_index_id,
        )?))
        .serve(addr)
        .await?;

    Ok(())
}
