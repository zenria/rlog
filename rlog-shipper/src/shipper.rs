use std::time::Duration;

use anyhow::Context;
use rlog_common::utils::format_error;
use rlog_grpc::{
    rlog_service_protocol::{log_collector_client::LogCollectorClient, LogLine},
    tonic::{transport::Endpoint, Code, Request, Response, Status},
};
use tokio::sync::mpsc::{channel, Sender};

pub fn launch_grpc_shipper(endpoint: Endpoint) -> Sender<LogLine> {
    let (sender, mut receiver) = channel(20_000);

    tokio::spawn(async move {
        loop {
            match async {
                tracing::info!("Connecting to collector");
                let channel = endpoint
                    .connect()
                    .await
                    .context("Unable to connect to gRPC endpoint")?;
                tracing::info!("Connected to collector");

                let mut client = LogCollectorClient::new(channel);

                while let Some(log_line) = receiver.recv().await {
                    // do something
                    let request = Request::new(log_line);
                    let response: Result<Response<()>, Status> = client.log(request).await;
                    if let Err(status) = response {
                        if status.code() == Code::Unavailable {
                            tracing::error!(
                                "Unable to send LogLine, collector unavailable: {}",
                                status.message()
                            );
                        } else {
                            // unhandled error
                            Err(status).context("unable to send log line to collector")?;
                        }
                    }
                }

                Ok::<_, anyhow::Error>(())
            }
            .await
            {
                Ok(_) => {
                    // this should not happen (by construction)
                    tracing::error!("log_line channel closed!");
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

    sender
}
