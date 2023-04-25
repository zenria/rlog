use config::CONFIG;
use forward_loop::{forward_loop, ForwardMetrics};
use futures::future::join_all;
use gelf_server::launch_gelf_server;
use grpc_out::launch_grpc_shipper;
use log_file::watch_log;
use metrics::{
    FILES_ERROR_COUNT, FILES_PROCESSED_COUNT, FILES_QUEUE_COUNT, GELF_ERROR_COUNT,
    GELF_PROCESSED_COUNT, GELF_QUEUE_COUNT, SHIPPER_QUEUE_COUNT, SYSLOG_ERROR_COUNT,
    SYSLOG_PROCESSED_COUNT, SYSLOG_QUEUE_COUNT,
};
use rlog_grpc::tonic::transport::Endpoint;
use syslog_server::launch_syslog_udp_server;
use tokio::{join, task::JoinHandle};
use tokio_util::sync::CancellationToken;

pub mod config;
mod forward_loop;
mod gelf_server;
mod generic_log;
mod grpc_out;
mod log_file;
mod metrics;
mod syslog_server;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

pub struct ServerConfig {
    pub grpc_collector_endpoint: Endpoint,
    pub syslog_udp_bind_address: String,
    pub gelf_tcp_bind_address: String,
}
pub struct ShipperServer {
    syslog_in: JoinHandle<()>,
    gelf_in: JoinHandle<()>,
    grpc_out: JoinHandle<()>,
    files_in: Vec<JoinHandle<()>>,
    shutdown_token: CancellationToken,
}
impl ShipperServer {
    pub async fn start_shipper_server(server_config: ServerConfig) -> anyhow::Result<Self> {
        let shutdown_token = CancellationToken::new();
        let gelf_receiver = launch_gelf_server(
            &server_config.gelf_tcp_bind_address,
            shutdown_token.child_token(),
        )
        .await?;

        let syslog_receiver = launch_syslog_udp_server(
            &server_config.syslog_udp_bind_address,
            shutdown_token.child_token(),
        )
        .await?;

        let (grpc_log_line_sender, grpc_out) = launch_grpc_shipper(
            server_config.grpc_collector_endpoint,
            shutdown_token.child_token(),
        );
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
        let mut files_in = Vec::new();
        for (path, _) in &CONFIG.load().files_in {
            files_in.push(tokio::spawn(forward_loop(
                watch_log(path, shutdown_token.child_token()).await?,
                grpc_log_line_sender.clone(),
                "files_in",
                ForwardMetrics {
                    in_queue_size: &FILES_QUEUE_COUNT,
                    in_processed_count: &FILES_PROCESSED_COUNT,
                    in_error_count: &FILES_ERROR_COUNT,
                    out_queue_size: &SHIPPER_QUEUE_COUNT,
                },
            )));
        }

        Ok(Self {
            syslog_in,
            gelf_in,
            grpc_out,
            files_in,
            shutdown_token,
        })
    }

    /// Gracefully shutdown the server, waiting for queues to empty
    pub async fn shutdown(self) {
        self.shutdown_token.cancel();
        let _ = join!(
            self.syslog_in,
            self.gelf_in,
            self.grpc_out,
            join_all(self.files_in)
        );
    }
}
