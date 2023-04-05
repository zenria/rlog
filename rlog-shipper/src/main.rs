use std::{str::FromStr, time::Duration};

use anyhow::Context;
use clap::Parser;
use futures::future;
use rlog_common::utils::read_file;
use rlog_grpc::{
    rlog_service_protocol::{
        log_collector_client::LogCollectorClient, log_line::Line, GelfLogLine, LogLine,
    },
    tonic::{
        transport::{Certificate, Channel, ClientTlsConfig, Identity, Uri},
        Request,
    },
};

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
    #[arg(long, env, default_value = "127.0.0.1:21055")]
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

    let channel = endpoint
        .connect()
        .await
        .context("Unable to connect to gRPC endpoint")?;

    let mut client = LogCollectorClient::new(channel);

    // let's send a test request

    let test_request = Request::new(futures::stream::once(future::ready(LogLine {
        host: hostname::get()?.to_string_lossy().to_string(),
        timestamp: None,
        line: Some(Line::Gelf(GelfLogLine {
            payload: "coucou les amis".to_string(),
        })),
    })));

    client.log(test_request).await?;

    Ok(())
}
