use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use rlog_collector::{config::CONFIG, CollectorServer, CollectorServerConfig};
use rlog_common::{
    config::setup_config_from_file,
    utils::{init_logging, read_file},
};
use rlog_grpc::tonic::transport::{Certificate, Identity, Server, ServerTlsConfig};
use tokio::{select, signal::unix::SignalKind};

use rlog_collector::metrics::launch_async_process_collector;

/// Collects logs locally and ship them to a remote destination
#[derive(Debug, Parser)]
struct Opts {
    /// trusted CA certificate used for mTLS connection
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

    /// Configuration file, if not provided, a minimal default configuration will be used
    #[arg(long, short, env)]
    config: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(e) = dotenv::dotenv() {
        eprintln!("WARN: unable to setup dotenv (.env files): {e}");
    };
    let opts = Opts::parse();

    init_logging();

    if let Some(path) = opts.config.as_ref() {
        setup_config_from_file(path, &CONFIG)?;
    }

    tracing::info!(
        "Starting rlog-collector {} with config:\n{}",
        rlog_collector::VERSION,
        serde_yaml::to_string(CONFIG.load().as_ref())?
    );

    launch_async_process_collector(Duration::from_millis(500));

    let server = Server::builder()
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

    let collector_server = CollectorServer::start_collector_server(CollectorServerConfig {
        http_status_bind_address: opts.http_status_bind_address,
        grpc_bind_address: opts.grpc_bind_address,
        quickwit_rest_url: opts.quickwit_rest_url,
        quickwit_index_id: opts.quickwit_index_id,
        server,
    })?;

    let mut sigterm = tokio::signal::unix::signal(SignalKind::terminate()).unwrap();
    select! {
        _ = tokio::signal::ctrl_c() => {
            tracing::debug!("CTRL-C PRESSED!");
        }
        _ = sigterm.recv() => {
            tracing::debug!("Received SIGTERM");
        }
    }
    tracing::info!("Request to shutdown received, initiating graceful shutdown.");
    collector_server.shutdown().await;
    tracing::info!("All tasks successfully exited!");
    Ok(())
}
