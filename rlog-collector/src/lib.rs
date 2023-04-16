use std::time::Duration;

use anyhow::Context;
use rlog_grpc::{
    rlog_service_protocol::log_collector_server::LogCollectorServer, tonic::transport::Server,
};
use tokio::{join, task::JoinHandle};
use tokio_util::sync::CancellationToken;

mod batch;
mod config;
mod grpc_server;
mod http_status_server;
mod index;
pub mod metrics;

pub struct CollectorServer {
    shutdown_token: CancellationToken,
    indexer_handle: JoinHandle<()>,
}

pub struct CollectorServerConfig {
    pub http_status_bind_address: String,
    pub grpc_bind_address: String,
    pub quickwit_rest_url: String,
    pub quickwit_index_id: String,
    pub server: Server,
}

impl CollectorServer {
    pub fn start_collector_server(config: CollectorServerConfig) -> anyhow::Result<Self> {
        http_status_server::launch_server(
            &config.http_status_bind_address,
            &config.quickwit_rest_url,
        )?;

        let shutdown_token = CancellationToken::new();

        let (log_sender, batch_log_receiver) = batch::launch_batch_collector(
            Duration::from_secs(1),
            100,
            shutdown_token.child_token(),
        );

        let indexer_handle = index::launch_index_loop(
            &config.quickwit_rest_url,
            &config.quickwit_index_id,
            batch_log_receiver,
        )?;
        let addr = config
            .grpc_bind_address
            .parse()
            .context("Invalid grpc bind address")?;

        tracing::info!("Starting rlog-collector gRPC server at {addr}");
        tokio::spawn(async move {
            let mut server = config.server;
            if let Err(e) = server
                .add_service(LogCollectorServer::new(
                    grpc_server::LogCollectorServer::new(log_sender),
                ))
                .serve(addr)
                .await
            {
                tracing::error!("Unable to launch gRPC server: {e}");
                std::process::exit(1);
            }
        });
        Ok(Self {
            shutdown_token,
            indexer_handle,
        })
    }

    pub async fn shutdown(self) {
        self.shutdown_token.cancel();
        // we only need to wait for the indexer task to terminate
        // the shutdown_token will properly terminate the batch task this will
        // - close the batch channel after laft batch
        // - close the send channel to the batch task, the server will
        //   always answer "unavailable" to shippers
        let _ = join!(self.indexer_handle);
    }
}
