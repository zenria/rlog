use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use futures::StreamExt;
use rlog_common::utils::read_file;
use rlog_grpc::{
    rlog_service_protocol::{log_collector_server::LogCollectorServer, LogLine},
    tonic::{
        self,
        transport::{Certificate, Identity, Server, ServerTlsConfig},
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

    #[arg(long, env)]
    grpc_bind_address: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Err(e) = dotenv::dotenv() {
        eprintln!("WARN: unable to setup dotenv (.env files): {e}");
    };
    let opts = Opts::parse();

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

    println!("Starting rlog-collector gRPC server at {addr}");

    server
        .add_service(LogCollectorServer::new(DummyLogCollectorServer))
        .serve(addr)
        .await?;

    Ok(())
}

struct DummyLogCollectorServer;

#[tonic::async_trait]
impl rlog_grpc::rlog_service_protocol::log_collector_server::LogCollector
    for DummyLogCollectorServer
{
    async fn log(
        &self,
        request: tonic::Request<tonic::Streaming<LogLine>>,
    ) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        let mut request_stream = request.into_inner();

        while let Some(log_line) = request_stream.next().await {
            let log_line = log_line?;
            println!("{log_line:?}");
        }
        Ok(tonic::Response::new(()))
    }
}
