use std::{sync::atomic::Ordering, time::Duration};

use anyhow::Context;
use futures::FutureExt;
use rlog_common::utils::format_error;
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
        let mut current_log_line = None;
        loop {
            match async {
                let mut client = match connect(&endpoint, &shutdown_token).await{
                    Some(client) => client,
                    None => return Ok(()),
                };

                let mut metrics_report_interval =
                    IntervalStream::new(interval(Duration::from_secs(30)));

                loop {
                    // send current log_line if any
                    if let Some(log_line) = current_log_line.take(){
                        SHIPPER_QUEUE_COUNT.fetch_sub(1, Ordering::Relaxed);
                        tracing::debug!("Will ship {log_line:#?}");
                        // do something
                        let request = Request::new(log_line);
                        let response: Result<Response<()>, Status> = client.log(request).await;
                        if let Err(status) = response {
                            SHIPPER_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
                            if status.code() == Code::Unavailable {
                                tracing::error!(
                                    "Unable to send LogLine, collector unavailable: {}",
                                    status.message()
                                );
                            } else {
                                // unhandled error
                                Err(status).context("unable to send log line to collector")?;
                            }
                        } else {
                            SHIPPER_PROCESSED_COUNT.fetch_add(1, Ordering::Relaxed);
                        }

                    }
                    select! {
                        _ = metrics_report_interval.next().fuse() => {
                            client.report_metrics(Request::new(to_grpc_metrics())).await.context("Unable to report metrics")?;
                        }
                        log_line = receiver.recv() => {
                            match log_line{
                                Some(log_line)=>  current_log_line = Some(log_line),
                                None => break,
                            }
                        }
                    }
                }

                Ok::<_, anyhow::Error>(())
            }
            .await
            {
                Ok(_) => {
                    // this should not happen (by construction)
                    tracing::info!("gelf_out channel closed, shutting down GELF output");
                    return;
                }
                Err(e) => {
                    tracing::error!(
                        "Error connecting/sending to gRPC collector: {}",
                        format_error(e)
                    );
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
