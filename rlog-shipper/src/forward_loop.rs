use rlog_common::utils::format_error;
use rlog_grpc::rlog_service_protocol::LogLine;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;

pub struct ForwardMetrics {
    pub in_queue_size: &'static AtomicU64,
    pub in_processed_count: &'static AtomicU64,
    pub in_error_count: &'static AtomicU64,
    pub out_queue_size: &'static AtomicU64,
}

pub async fn forward_loop<T>(
    mut input: Receiver<T>,
    grpc_out: Sender<LogLine>,
    input_name: &str,
    fw_metrics: ForwardMetrics,
) where
    LogLine: TryFrom<T, Error = anyhow::Error>,
{
    while let Some(syslog) = input.recv().await {
        fw_metrics.in_queue_size.fetch_sub(1, Ordering::Relaxed);
        fw_metrics
            .in_processed_count
            .fetch_add(1, Ordering::Relaxed);
        // construct a valid LogLine from gelf stuff
        let log_line = match LogLine::try_from(syslog) {
            Ok(l) => l,
            Err(e) => {
                fw_metrics.in_error_count.fetch_add(1, Ordering::Relaxed);
                tracing::error!(
                    "received an invalid log from {input_name}! {}",
                    format_error(e)
                );
                continue;
            }
        };
        // if the channel is full, is will block here ; filling channels from each
        // server (syslog & gelf), when those channel will be full, new messages will be discarded
        if let Err(e) = grpc_out.send(log_line).await {
            tracing::error!("Channel closed! {e}");
            break;
        } else {
            fw_metrics.out_queue_size.fetch_add(1, Ordering::Relaxed);
        }
    }
    tracing::info!("{input_name} input channel closed, {input_name} forward task stopped.");
}
