use std::{str::FromStr, time::Duration};

use anyhow::Context;
use clap::Parser;
use gelf_server::launch_gelf_server;
use rlog_common::utils::{format_error, init_logging, read_file};
use rlog_grpc::{
    rlog_service_protocol::LogLine,
    tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity, Uri},
};
use shipper::launch_grpc_shipper;
use syslog_server::launch_syslog_udp_server;
use tracing::Instrument;

mod gelf_server;
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(e) = dotenv::dotenv() {
        eprintln!("WARN: unable to setup dotenv (.env files): {e}");
    };
    let opts = Opts::parse();

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

    init_logging();

    let mut gelf_receiver = launch_gelf_server(&opts.gelf_tcp_bind_address).await?;

    let mut syslog_receiver = launch_syslog_udp_server(&opts.syslog_udp_bind_address).await?;

    let grpc_log_line_sender = launch_grpc_shipper(endpoint);

    async move {
        while let Some(gelf_log) = gelf_receiver.recv().await {
            // construct a valid LogLine from gelf stuff
            let log_line = match LogLine::try_from(gelf_log) {
                Ok(l) => l,
                Err(e) => {
                    tracing::error!("received an invalid GELF log! {}", format_error(e));
                    continue;
                }
            };
            // if the channel is full, is will block here ; filling channels from each
            // server (syslog & gelf), when those channel will be full, new messages will be discarded
            if let Err(e) = grpc_log_line_sender.send(log_line).await {
                tracing::error!("Channel closed! {e}");
                break;
            }
        }
        tracing::error!("log_forward_loop exited!");
    }
    .instrument(tracing::info_span!("log_forward_loop"))
    .await;

    Ok(())
}
