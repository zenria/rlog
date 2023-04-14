use std::{str::FromStr, sync::atomic::Ordering, time::Duration};

use anyhow::Context;
use clap::Parser;
use config::setup_config_from_file;
use gelf_server::launch_gelf_server;
use rlog_common::utils::{format_error, init_logging, read_file};
use rlog_grpc::{
    rlog_service_protocol::LogLine,
    tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity, Uri},
};
use shipper::launch_grpc_shipper;
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
mod gelf_server;
mod metrics;
mod shipper;
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

    let mut gelf_receiver =
        launch_gelf_server(&opts.gelf_tcp_bind_address, shutdown_token.child_token()).await?;

    let mut syslog_receiver =
        launch_syslog_udp_server(&opts.syslog_udp_bind_address, shutdown_token.child_token())
            .await?;

    let (grpc_log_line_sender, grpc_out) = launch_grpc_shipper(endpoint);

    let gelf_in = tokio::spawn({
        let grpc_log_line_sender = grpc_log_line_sender.clone();
        async move {
            while let Some(gelf_log) = gelf_receiver.recv().await {
                GELF_QUEUE_COUNT.fetch_sub(1, Ordering::Relaxed);
                GELF_PROCESSED_COUNT.fetch_add(1, Ordering::Relaxed);
                // construct a valid LogLine from gelf stuff
                let log_line = match LogLine::try_from(gelf_log) {
                    Ok(l) => l,
                    Err(e) => {
                        GELF_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
                        tracing::error!("received an invalid GELF log! {}", format_error(e));
                        continue;
                    }
                };
                // if the channel is full, is will block here ; filling channels from each
                // server (syslog & gelf), when those channel will be full, new messages will be discarded
                if let Err(e) = grpc_log_line_sender.send(log_line).await {
                    tracing::error!("Channel closed! {e}");
                    break;
                } else {
                    SHIPPER_QUEUE_COUNT.fetch_add(1, Ordering::Relaxed);
                }
            }
            tracing::info!("GELF input channel closed, GELF forward task stopped.");
        }
    });
    let syslog_in = tokio::spawn(async move {
        while let Some(syslog) = syslog_receiver.recv().await {
            SYSLOG_QUEUE_COUNT.fetch_sub(1, Ordering::Relaxed);
            SYSLOG_PROCESSED_COUNT.fetch_add(1, Ordering::Relaxed);
            // construct a valid LogLine from gelf stuff
            let log_line = match LogLine::try_from(syslog) {
                Ok(l) => l,
                Err(e) => {
                    SYSLOG_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
                    tracing::error!("received an invalid Syslog log! {}", format_error(e));
                    continue;
                }
            };
            // if the channel is full, is will block here ; filling channels from each
            // server (syslog & gelf), when those channel will be full, new messages will be discarded
            if let Err(e) = grpc_log_line_sender.send(log_line).await {
                tracing::error!("Channel closed! {e}");
                break;
            } else {
                SHIPPER_QUEUE_COUNT.fetch_add(1, Ordering::Relaxed);
            }
        }
        tracing::info!("Syslog input channel closed, Syslog forward task stopped.");
    });

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
