use std::{collections::HashMap, sync::atomic::Ordering};

use anyhow::Context;
use bytes::BytesMut;
use rlog_grpc::rlog_service_protocol::{GelfLogLine, LogLine};
use serde_json::Value;
use tokio::{
    io::AsyncReadExt,
    net::TcpListener,
    sync::mpsc::{channel, error::TrySendError, Receiver},
};
use tracing::Instrument;

use crate::metrics::{GELF_ERROR_COUNT, GELF_QUEUE_COUNT};

pub struct GelfLog(serde_json::Value);

impl GelfLog {
    pub fn to_json(&self) -> String {
        self.0.to_string()
    }
}

pub async fn launch_gelf_server(bind_address: &str) -> anyhow::Result<Receiver<GelfLog>> {
    let (sender, receiver) = channel(10000);

    let listener = TcpListener::bind(bind_address)
        .await
        .context("Unable to bind to GELF bind address")?;

    tracing::info!("GELF TCP server listening at {bind_address}");

    tokio::spawn(async move {
        loop {
            let (mut socket, r) = match listener.accept().await {
                Ok(connection) => connection,
                Err(e) => {
                    tracing::error!("Unable to accept incoming connection! {e}");
                    return;
                }
            };
            let sender = sender.clone();
            let remote_addr = format!("{r}");
            println!("Connected from {r}");
            tokio::spawn(
                async move {
                    tracing::info!("new connection");
                    let mut buffer = BytesMut::with_capacity(4096);
                    // In a loop, read data from the socket and write the data back.
                    loop {
                        let _n = match socket.read_buf(&mut buffer).await {
                            // graceful shutdown
                            Ok(n) if n == 0 && buffer.len() == 0 => break,
                            // connection closed during transmission of a frame
                            Ok(n) if n == 0 => {
                                tracing::error!("Connection reset by peer");
                                break;
                            }
                            Ok(n) => n,
                            Err(e) => {
                                tracing::error!("failed to read from socket; {e}");
                                return;
                            }
                        };
                        // check we received a \0 bytes indicating the end of a frame
                        while let Some(i) = buffer
                            .iter()
                            .enumerate()
                            .find(|(_i, byte)| byte == &&0)
                            .map(|(i, _)| i)
                        {
                            // there is a message between cursor & i
                            match serde_json::from_slice::<Value>(&buffer[0..i]) {
                                Ok(valid_json) => {
                                    tracing::debug!("Received: {valid_json}");

                                    if let Err(e) = sender.try_send(GelfLog(valid_json)) {
                                        GELF_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
                                        match e {
                                            TrySendError::Full(value) => {
                                                tracing::error!(
                                                    "Send buffer full: discarding value {}",
                                                    value.to_json()
                                                );
                                            }
                                            TrySendError::Closed(value) => {
                                                // this is not possible by construction...
                                                tracing::error!(
                                                    "Channel closed, discarding value {}",
                                                    value.to_json()
                                                );
                                            }
                                        }
                                        return;
                                    } else {
                                        GELF_QUEUE_COUNT.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Unable to decode json: {e}")
                                }
                            }
                            // remove the first part of the buffer and discard it
                            let _ = buffer.split_to(i + 1);
                        }
                    }
                    tracing::info!("Connection closed.");
                }
                .instrument(tracing::info_span!("gelf_conn_handler", remote_addr)),
            );
        }
    });

    Ok(receiver)
}

impl TryFrom<GelfLog> for LogLine {
    type Error = anyhow::Error;

    fn try_from(value: GelfLog) -> Result<Self, Self::Error> {
        let json = value.0;
        let json_map = json
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("{json} is not an object!"))?;
        // extract host & timestamp
        let hostname = json_map
            .get("host")
            .map(|v| v.as_str())
            .flatten()
            .ok_or_else(|| anyhow::anyhow!("{json} does not have a `host` string field!"))?;
        let timestamp_secs = json_map
            .get("timestamp")
            .map(|v| v.as_f64())
            .flatten()
            .ok_or_else(|| anyhow::anyhow!("{json} does not have a `timestamp` number field!"))?;
        // some gelf enabled software (java) sends timestamp with millis...
        let timestamp_millis = (timestamp_secs * 1000.0) as i64;
        let timestamp_secs = timestamp_millis / 1000;
        let nanos = ((timestamp_millis - timestamp_secs * 1000) * 1_000_000) as i32;

        let timestamp = rlog_grpc::prost_wkt_types::Timestamp {
            seconds: timestamp_secs,
            nanos,
        };

        let severity = json_map
            .get("level")
            .map(|v| v.as_i64())
            .flatten()
            .map(|v| v as i32)
            .unwrap_or(1); // ALERT by GELF spec

        let short_message = json_map
            .get("short_message")
            .map(|v| v.as_str())
            .flatten()
            .ok_or_else(|| {
                anyhow::anyhow!("{json} does not have a `short_message` string field!")
            })?;
        let full_message = json_map
            .get("full_message")
            .map(|v| v.as_str())
            .flatten()
            .map(ToString::to_string);
        let mut extra = HashMap::new();
        for (key, value) in json_map {
            let key = if key.starts_with('_') {
                &key[1..]
            } else {
                key.as_str()
            };
            match key {
                // ignore fields set elsewhere
                "host" | "timestamp" | "facility" | "version" | "level" | "short_message"
                | "full_message" => continue,
                _ => {}
            }
            extra.insert(key, value);
        }
        let extra = serde_json::to_string(&extra)?; // this cannot fail

        Ok(LogLine {
            host: hostname.into(),
            timestamp: Some(timestamp),
            line: Some(rlog_grpc::rlog_service_protocol::log_line::Line::Gelf(
                GelfLogLine {
                    short_message: short_message.into(),
                    full_message: full_message,
                    severity,
                    extra,
                },
            )),
        })
    }
}
