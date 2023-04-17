use std::{collections::HashMap, time::Duration};

use anyhow::{anyhow, Context};
use itertools::Itertools;
use reqwest::{Client, StatusCode, Url};
use rlog_grpc::{rlog_service_protocol::LogLine, OTELSeverity};
use serde::{Deserialize, Serialize};
use tokio::{sync::mpsc::Receiver, task::JoinHandle};

use crate::metrics::{
    COLLECTOR_INDEXED_COUNT, COLLECTOR_OUTPUT_COUNT, OUTPUT_STATUS_ERROR_LABEL_VALUE,
    OUTPUT_STATUS_OK_LABEL_VALUE, OUTPUT_STATUS_TOO_MANY_REQUEST_LABEL_VALUE,
    OUTPUT_SYSTEM_QUICKWIT_LABEL_VALUE,
};

#[derive(Serialize, Debug)]
#[serde(rename_all = "lowercase")]
enum LogSystem {
    Syslog,
    Gelf,
}

/// What is being indexed by quickwit
#[derive(Serialize, Debug)]
pub struct IndexLogEntry {
    message: String,
    /// timestamp: number of
    /// - seconds from EPOCH
    /// - milliseconds from EPOCh
    /// - microseconds from EPOCH
    /// - nanosecondes from EPOCH
    /// Quickwit will detect the right format...
    timestamp: u64,
    hostname: String,
    service_name: String,
    severity_text: String,
    /// open telemetry severity
    severity_number: u64,

    log_system: LogSystem,

    #[serde(flatten)]
    free_fields: HashMap<String, serde_json::Value>,
}

pub fn launch_index_loop(
    quickwit_rest_url: &str,
    index_id: &str,
    mut batch_receiver: Receiver<Vec<IndexLogEntry>>,
) -> anyhow::Result<JoinHandle<()>> {
    // parse url & setup http client
    let quickwit_rest_url: Url = quickwit_rest_url
        .parse()
        .context("invalid quickwit REST url")?;
    let ingest_url = quickwit_rest_url.join(&format!("api/v1/{index_id}/ingest"))?;
    let http_client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()?;

    Ok(tokio::spawn(async move {
        let mut batch_to_send: Option<String> = None;
        let mut batch_count = 0;
        loop {
            if let Some(body) = batch_to_send.take() {
                tracing::debug!("Sending to quickwit {batch_count} items:\n{body}");
                // send the stuff
                match http_client
                    .post(ingest_url.clone())
                    .body(body.clone())
                    .send()
                    .await
                {
                    Ok(quickwit_response) => {
                        match quickwit_response.status() {
                            StatusCode::OK => {
                                // consume response
                                let _response = quickwit_response.text().await;
                                tracing::debug!("OK");
                                COLLECTOR_INDEXED_COUNT.inc_by(batch_count);
                                COLLECTOR_OUTPUT_COUNT
                                    .with_label_values(&[
                                        OUTPUT_SYSTEM_QUICKWIT_LABEL_VALUE,
                                        OUTPUT_STATUS_OK_LABEL_VALUE,
                                    ])
                                    .inc();
                                // nothing to do here, this has been successfully accepted by quickwit
                            }
                            StatusCode::TOO_MANY_REQUESTS => {
                                // consume response
                                let _response = quickwit_response.text().await;
                                tracing::warn!(
                                    "Quickwit overloaded (429), wait 5 seconds before retrying"
                                );
                                batch_to_send = Some(body);
                                COLLECTOR_OUTPUT_COUNT
                                    .with_label_values(&[
                                        OUTPUT_SYSTEM_QUICKWIT_LABEL_VALUE,
                                        OUTPUT_STATUS_TOO_MANY_REQUEST_LABEL_VALUE,
                                    ])
                                    .inc();
                                tokio::time::sleep(Duration::from_secs(5)).await;
                                continue;
                            }
                            other => {
                                let response = quickwit_response.text().await;
                                tracing::error!("Unhandled status code {other} - {response:?}");
                                // retry batch
                                batch_to_send = Some(body);
                                COLLECTOR_OUTPUT_COUNT
                                    .with_label_values(&[
                                        OUTPUT_SYSTEM_QUICKWIT_LABEL_VALUE,
                                        OUTPUT_STATUS_ERROR_LABEL_VALUE,
                                    ])
                                    .inc();
                                tokio::time::sleep(Duration::from_secs(1)).await;
                                continue;
                            }
                        }
                    }
                    Err(quickwit_error) => {
                        // connect error or some low level error, we must retry
                        tracing::error!(
                            "Error sending batch to quickwit, retry in 1s - {quickwit_error}"
                        );
                        batch_to_send = Some(body);
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                }
            }
            match batch_receiver.recv().await {
                Some(batch) => {
                    batch_count = batch.len() as u64;
                    let body = batch
                        .into_iter()
                        .map(|j| serde_json::to_string(&j).unwrap())
                        .join("\n");
                    batch_to_send = Some(body);
                }
                // channel close (server shutdown)
                None => break,
            }
        }
    }))
}

#[derive(Deserialize)]
#[allow(unused)]
struct QuickwitIngestResponse {
    num_docs_for_processing: u64,
}

impl TryFrom<LogLine> for IndexLogEntry {
    type Error = anyhow::Error;

    fn try_from(value: LogLine) -> Result<Self, Self::Error> {
        let hostname = value.host;
        let timestamp = value
            .timestamp
            .ok_or(anyhow!("`timestamp` field is mandatory"))?;
        let line = value.line.ok_or(anyhow!("`line` field is mandatory"))?;

        match line {
            rlog_grpc::rlog_service_protocol::log_line::Line::Gelf(gelf) => {
                let severity = OTELSeverity::from(gelf.severity());
                let message = gelf.full_message.unwrap_or(gelf.short_message);
                let mut extra: HashMap<String, serde_json::Value> =
                    serde_json::from_str(&gelf.extra)
                        .context("`extra` field is not a valid json object")?;
                let service_name = extra
                    .remove("service")
                    .map(|s| s.as_str().map(|s| s.to_string()))
                    .flatten()
                    .unwrap_or_else(|| "unknown".to_string());
                let severity_text = severity.to_string();
                let severity_number = severity as u8;
                let timestamp_ms = timestamp.seconds * 1000 + (timestamp.nanos as i64) / 1_000_000;
                Ok(IndexLogEntry {
                    message,
                    timestamp: timestamp_ms as u64,
                    hostname,
                    service_name,
                    severity_text,
                    severity_number: severity_number as u64,
                    log_system: LogSystem::Gelf,
                    free_fields: extra,
                })
            }
            rlog_grpc::rlog_service_protocol::log_line::Line::Syslog(syslog) => {
                let severity = OTELSeverity::from(syslog.severity());
                let severity_text = severity.to_string();
                let severity_number = severity as u8;

                let mut free_fields: HashMap<String, serde_json::Value> = HashMap::new();
                free_fields.insert("facility".into(), syslog.facility().as_str_name().into());
                if let Some(pid) = syslog.proc_pid {
                    free_fields.insert("proc_pid".into(), pid.into());
                }
                if let Some(proc_name) = syslog.proc_name {
                    free_fields.insert("proc_name".into(), proc_name.into());
                }
                if let Some(msgid) = syslog.msgid {
                    free_fields.insert("msgid".into(), msgid.into());
                }
                let message = syslog.msg;
                let service_name = syslog.appname.unwrap_or_else(|| "_syslog".into());
                let timestamp_ms = timestamp.seconds * 1000 + (timestamp.nanos as i64) / 1_000_000;

                Ok(IndexLogEntry {
                    message,
                    timestamp: timestamp_ms as u64,
                    hostname,
                    service_name,
                    severity_text,
                    severity_number: severity_number as u64,
                    log_system: LogSystem::Syslog,
                    free_fields,
                })
            }
        }
    }
}
