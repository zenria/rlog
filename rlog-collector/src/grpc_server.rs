use rlog_common::utils::format_error;
use rlog_grpc::{
    rlog_service_protocol::{LogLine, Metrics},
    tonic::{self, async_trait, Status},
};
use serde::Serialize;
use tokio::sync::mpsc::Sender;
use tracing::instrument;

use crate::{
    http_status_server::report_connected_host,
    index::IndexLogEntry,
    metrics::{SHIPPER_ERROR_COUNT, SHIPPER_PROCESSED_COUNT, SHIPPER_QUEUE_COUNT},
};

#[derive(Serialize, Debug)]
#[serde(rename_all = "lowercase")]
enum LogSystem {
    Syslog,
    Gelf,
}

pub struct LogCollectorServer {
    /// each IndexLogEntry will be sent here
    sender: Sender<IndexLogEntry>,
}

impl LogCollectorServer {
    pub fn new(sender: Sender<IndexLogEntry>) -> Self {
        Self { sender }
    }
}
#[async_trait]
impl rlog_grpc::rlog_service_protocol::log_collector_server::LogCollector for LogCollectorServer {
    #[instrument(skip(self, request))]
    async fn log(
        &self,
        request: tonic::Request<LogLine>,
    ) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        let log_line = request.into_inner();

        tracing::debug!("Received {log_line:#?}");

        let log_entry = IndexLogEntry::try_from(log_line)
            // Reject the request if the received LogLine is invalid
            .map_err(|e| {
                Status::invalid_argument(format!("Invalid LogLine {}", format_error(e)))
            })?;

        tracing::debug!("Converted to {log_entry:#?}");

        if let Err(e) = self.sender.send(log_entry).await {
            Err(tonic::Status::unavailable("shutdown in progress"))
        } else {
            Ok(tonic::Response::new(()))
        }
    }
    #[instrument(skip(self, request))]
    async fn report_metrics(
        &self,
        request: tonic::Request<Metrics>,
    ) -> std::result::Result<tonic::Response<()>, tonic::Status> {
        let metrics = request.into_inner();
        tracing::debug!("{metrics:#?}");
        report_connected_host(&metrics.hostname).await;

        for (queue_name, count) in metrics.queue_count {
            SHIPPER_QUEUE_COUNT
                .get_metric_with_label_values(&[&metrics.hostname, &queue_name])
                .unwrap()
                .set(count as i64);
        }

        for (queue_name, count) in metrics.processed_count {
            let counter = SHIPPER_PROCESSED_COUNT
                .get_metric_with_label_values(&[&metrics.hostname, &queue_name])
                .unwrap();
            let current = counter.get();
            if count > current {
                counter.inc_by(count - current);
            } else {
                counter.reset();
                counter.inc_by(count);
            }
        }
        for (queue_name, count) in metrics.error_count {
            let counter = SHIPPER_ERROR_COUNT
                .get_metric_with_label_values(&[&metrics.hostname, &queue_name])
                .unwrap();
            let current = counter.get();
            if count > current {
                counter.inc_by(count - current);
            } else {
                counter.reset();
                counter.inc_by(count);
            }
        }

        Ok(tonic::Response::new(()))
    }
}
