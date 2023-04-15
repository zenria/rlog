use std::{str::FromStr, time::Duration};

use anyhow::Context;
use clap::Parser;
use config::setup_config_from_file;
use forward_loop::{forward_loop, ForwardMetrics};
use gelf_server::launch_gelf_server;
use grpc_out::launch_grpc_shipper;
use rlog_common::utils::{init_logging, read_file};
use rlog_grpc::tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity, Uri};
use syslog_server::launch_syslog_udp_server;
use tokio::{join, select, signal::unix::SignalKind};
use tokio_util::sync::CancellationToken;

use crate::{
    config::CONFIG,
    metrics::{
        GELF_ERROR_COUNT, GELF_PROCESSED_COUNT, GELF_QUEUE_COUNT, SHIPPER_QUEUE_COUNT,
        SYSLOG_ERROR_COUNT, SYSLOG_PROCESSED_COUNT, SYSLOG_QUEUE_COUNT,
    },
};

mod config;
mod forward_loop;
mod gelf_server;
mod grpc_out;
mod metrics;
mod syslog_server;

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
    /// Remote server hostname, if present it will be used for remote
    /// server identify verification (SNI) instead of the host part
    /// of the gRPC collector URL.
    #[arg(long, env)]
    tls_remote_hostname: Option<String>,

    /// URL of the gRPC endpoint that collects logs
    #[arg(long, env)]
    grpc_collector_url: String,

    /// syslog udp protocol bind address
    #[arg(long, env, default_value = "127.0.0.1:21054")]
    syslog_udp_bind_address: String,
    /// gelf tcp protocol bind address
    #[arg(long, env, default_value = "127.0.0.1:12201")]
    gelf_tcp_bind_address: String,

    /// Configuration file, if not provided, a minimal default configuration will be used
    #[arg(long, short, env)]
    config: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let shutdown_token = CancellationToken::new();

    if let Err(e) = dotenv::dotenv() {
        eprintln!("WARN: unable to setup dotenv (.env files): {e}");
    };

    init_logging();

    let opts = Opts::parse();

    if let Some(path) = opts.config.as_ref() {
        setup_config_from_file(path)?;
    }

    tracing::info!(
        "Starting rlog-shipper with config:\n{}",
        serde_yaml::to_string(CONFIG.load().as_ref())?
    );

    let endpoint = Channel::builder(
        Uri::from_str(&opts.grpc_collector_url)
            .with_context(|| format!("cannot parse {}", opts.grpc_collector_url))?,
    )
    // always setup tcp keepalive
    .tcp_keepalive(Some(Duration::from_secs(60)))
    // tls config
    .tls_config({
        let mut client_tls_config = ClientTlsConfig::new();
        client_tls_config = client_tls_config
            .identity(Identity::from_pem(
                read_file(&opts.tls_certificate).context("Cannot open certificate")?,
                read_file(&opts.tls_private_key).context("Cannot open private key")?,
            ))
            .ca_certificate(Certificate::from_pem(
                read_file(&opts.tls_ca_certificate).context("Cannot open ca certificate")?,
            ));
        if let Some(hostname) = &opts.tls_remote_hostname {
            client_tls_config = client_tls_config.domain_name(hostname);
        }
        Ok::<_, anyhow::Error>(client_tls_config)
    }?)
    .context("Invalid TLS configuration")?;

    let gelf_receiver =
        launch_gelf_server(&opts.gelf_tcp_bind_address, shutdown_token.child_token()).await?;

    let syslog_receiver =
        launch_syslog_udp_server(&opts.syslog_udp_bind_address, shutdown_token.child_token())
            .await?;

    let (grpc_log_line_sender, grpc_out) = launch_grpc_shipper(endpoint);
    let gelf_in = tokio::spawn(forward_loop(
        gelf_receiver,
        grpc_log_line_sender.clone(),
        "gelf_in",
        ForwardMetrics {
            in_queue_size: &GELF_QUEUE_COUNT,
            in_processed_count: &GELF_PROCESSED_COUNT,
            in_error_count: &GELF_ERROR_COUNT,
            out_queue_size: &SHIPPER_QUEUE_COUNT,
        },
    ));

    let syslog_in = tokio::spawn(forward_loop(
        syslog_receiver,
        grpc_log_line_sender.clone(),
        "syslog_in",
        ForwardMetrics {
            in_queue_size: &SYSLOG_QUEUE_COUNT,
            in_processed_count: &SYSLOG_PROCESSED_COUNT,
            in_error_count: &SYSLOG_ERROR_COUNT,
            out_queue_size: &SHIPPER_QUEUE_COUNT,
        },
    ));
    drop(grpc_log_line_sender);

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
    shutdown_token.cancel();

    let _ = join!(syslog_in, gelf_in, grpc_out);
    tracing::info!("All tasks successfully exited!");
    Ok(())
}
