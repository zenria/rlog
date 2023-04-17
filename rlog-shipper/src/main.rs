use std::{str::FromStr, time::Duration};

use anyhow::Context;
use clap::Parser;
use rlog_common::{
    config::setup_config_from_file,
    utils::{init_logging, read_file},
};
use rlog_grpc::tonic::transport::{Certificate, Channel, ClientTlsConfig, Identity, Uri};
use rlog_shipper::{config::CONFIG, ServerConfig, ShipperServer};
use tokio::{select, signal::unix::SignalKind};

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
    if let Err(e) = dotenv::dotenv() {
        eprintln!("WARN: unable to setup dotenv (.env files): {e}");
    };

    init_logging();

    let opts = Opts::parse();

    if let Some(path) = opts.config.as_ref() {
        setup_config_from_file(path, &CONFIG)?;
    }

    tracing::info!(
        "Starting rlog-shipper {} with config:\n{}",
        rlog_shipper::VERSION,
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

    let shipper_server = ShipperServer::start_shipper_server(ServerConfig {
        grpc_collector_endpoint: endpoint,
        syslog_udp_bind_address: opts.syslog_udp_bind_address,
        gelf_tcp_bind_address: opts.gelf_tcp_bind_address,
    })
    .await?;

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
    shipper_server.shutdown().await;

    tracing::info!("All tasks successfully exited!");
    Ok(())
}
