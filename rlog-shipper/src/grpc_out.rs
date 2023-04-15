use std::{sync::atomic::Ordering, time::Duration};

use anyhow::Context;
use futures::{ FutureExt};
use rlog_common::utils::format_error;
use rlog_grpc::{
    rlog_service_protocol::{log_collector_client::LogCollectorClient, LogLine},
    tonic::{transport::Endpoint, Code, Request, Response, Status},
};
use tokio::{
    sync::mpsc::{channel, Sender},
    time::interval,select, task::JoinHandle
};
use tokio_stream::{wrappers::IntervalStream, StreamExt};

use crate::{metrics::{SHIPPER_PROCESSED_COUNT, SHIPPER_QUEUE_COUNT, to_grpc_metrics, SHIPPER_ERROR_COUNT}, config::CONFIG};

pub fn launch_grpc_shipper(endpoint: Endpoint) -> (Sender<LogLine>, JoinHandle<()>) {
    let (sender, mut receiver) = channel(CONFIG.load().grpc_out.max_buffer_size);


    let handle = tokio::spawn(async move {
        loop {
            match async {
                tracing::info!("Connecting to collector");
                let channel = endpoint
                    .connect()
                    .await
                    .context("Unable to connect to gRPC endpoint")?;
                tracing::info!("Connected to collector");

                let mut client = LogCollectorClient::new(channel);

                let mut metrics_report_interval =
                    IntervalStream::new(interval(Duration::from_secs(30)));

                loop {
                    select! {
                        _ = metrics_report_interval.next().fuse() => {
                            client.report_metrics(Request::new(to_grpc_metrics())).await.context("Unable to report metrics")?;
                        }
                        log_line = receiver.recv() => {
                            match log_line{
                                Some(log_line)=> {
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
                                None => {
                                    break;
                                }
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
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    });

    (sender,handle)
}
