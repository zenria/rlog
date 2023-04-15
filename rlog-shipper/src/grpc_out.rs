use std::{sync::atomic::Ordering, time::Duration};

use rlog_grpc::{
    rlog_service_protocol::{log_collector_client::LogCollectorClient, LogLine},
    tonic::{
        transport::{Channel, Endpoint},
        Code, Request, Response, Status,
    },
};
use tokio::{
    select,
    sync::mpsc::{channel, Sender},
    task::JoinHandle,
    time::interval,
};
use tokio_stream::{wrappers::IntervalStream, StreamExt};
use tokio_util::sync::CancellationToken;

use crate::{
    config::CONFIG,
    metrics::{to_grpc_metrics, SHIPPER_ERROR_COUNT, SHIPPER_PROCESSED_COUNT, SHIPPER_QUEUE_COUNT},
};

pub fn launch_grpc_shipper(
    endpoint: Endpoint,
    shutdown_token: CancellationToken,
) -> (Sender<LogLine>, JoinHandle<()>) {
    let (sender, mut receiver) = channel(CONFIG.load().grpc_out.max_buffer_size);

    let handle = tokio::spawn(async move {
        let mut current_log_line: Option<LogLine> = None;

        // Connect to remote endpoint
        //
        // Note: once connected, if it gets disconnected, tonic will try to reconnect in background
        // there is no need to manually reconnect it.
        //
        // This is utterly odd: the tonic api answer as if the remote endpoint sent a "Unavailable"
        // code. Not sure if it's a gRPC idiom but it is very confusing.

        let mut client = match connect(&endpoint, &shutdown_token).await {
            Some(client) => client,
            None => return,
        };

        let mut metrics_report_interval = IntervalStream::new(interval(Duration::from_secs(30)));

        loop {
            if shutdown_token.is_cancelled() {
                // early return to allow to exit if a log is being retried with a dead collector
                return;
            }
            // send current log_line if any
            if let Some(log_line) = current_log_line.take() {
                SHIPPER_QUEUE_COUNT.fetch_sub(1, Ordering::Relaxed);
                tracing::debug!("Will ship {log_line:#?}");
                // do something
                let request = Request::new(log_line.clone());
                let response: Result<Response<()>, Status> = client.log(request).await;
                if let Err(status) = response {
                    SHIPPER_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
                    match status.code() {
                        Code::InvalidArgument => {
                            // invalid log_line, no need to disconnect nor trying to re-send it
                            tracing::error!(
                                "Unable to send LogLine, collector responded invalid_argument: {} --- {log_line:?}",
                                status.message()
                            );
                        }
                        Code::OutOfRange => {
                            // this happens when the message is too large
                            tracing::error!(
                                "Unable to send LogLine, collector responded out_of_range, ignoring the log_line: {}",
                                status.message()
                            );
                        }
                        // this covers:
                        // - unavailable upstream (collector reports Unavailable)
                        // - disconnected collector, tonic api report Unaavailble and tries to reconnect
                        //   on the background
                        _ => {
                            tracing::error!(
                                "Unable to send LogLine, collector reported an error: {} - {status:?}",
                                status.message()
                            );
                            // collector unavailable means the upstream (quickwit) is not available
                            // wait a bit before trying to send again the log line
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            current_log_line = Some(log_line);
                            continue;
                        }
                    }
                } else {
                    SHIPPER_PROCESSED_COUNT.fetch_add(1, Ordering::Relaxed);
                }
            }
            select! {
                _ = metrics_report_interval.next() => {
                    if let Err(e) = client.report_metrics(Request::new(to_grpc_metrics())).await{
                        tracing::error!("Unable to report metrics: {e}");
                    }
                }
                log_line = receiver.recv() => {
                    match log_line{
                        Some(log_line)=>  current_log_line = Some(log_line),
                        None => break,
                    }
                }
            }
        }
    });

    (sender, handle)
}

async fn connect(
    endpoint: &Endpoint,
    shutdown_token: &CancellationToken,
) -> Option<LogCollectorClient<Channel>> {
    loop {
        tracing::info!("Connecting to collector");
        match endpoint.connect().await.map(LogCollectorClient::new) {
            Ok(client) => {
                tracing::info!("Connected to collector");
                return Some(client);
            }
            Err(e) => {
                tracing::error!("Unable to connect to collector gRPC endpoint: {e}");
                tokio::time::sleep(Duration::from_secs(1)).await;
                if shutdown_token.is_cancelled() {
                    // shutdown initiated, stop connection process
                    return None;
                }
            }
        }
    }
}
