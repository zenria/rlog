use std::{collections::HashMap, time::Duration};

use anyhow::{anyhow, Context};
use futures::TryFutureExt;
use reqwest::{Client, Url};
use rlog_common::utils::format_error;
use rlog_grpc::{
    rlog_service_protocol::{LogLine, Metrics},
    tonic::{self, async_trait, Status},
    OTELSeverity,
};
use serde::{Deserialize, Serialize};
use tracing::instrument;

use crate::{
    http_status_server::report_connected_host,
    metrics::{
        COLLECTOR_OUTPUT_COUNT, OUTPUT_STATUS_ERROR_LABEL_VALUE, OUTPUT_STATUS_OK_LABEL_VALUE,
        OUTPUT_SYSTEM_QUICKWIT_LABEL_VALUE, SHIPPER_ERROR_COUNT, SHIPPER_PROCESSED_COUNT,
        SHIPPER_QUEUE_COUNT,
    },
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
    timestamp_secs: u64,
    timestamp_nanos: u64,
    hostname: String,
    service_name: String,
    severity_text: String,
    /// open telemetry severity
    severity_number: u64,

    log_system: LogSystem,

    #[serde(flatten)]
    free_fields: HashMap<String, serde_json::Value>,
}

pub struct IndexLogCollectorServer {
    ingest_url: Url,
    http_client: Client,
}

impl IndexLogCollectorServer {
    pub fn new(quickwit_rest_url: &str, index_id: &str) -> anyhow::Result<Self> {
        let quickwit_rest_url: Url = quickwit_rest_url
            .parse()
            .context("invalid quickwit REST url")?;
        let ingest_url = quickwit_rest_url.join(&format!("api/v1/{index_id}/ingest"))?;
        let http_client = Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .build()?;
        Ok(Self {
            ingest_url,
            http_client,
        })
    }
}
#[async_trait]
impl rlog_grpc::rlog_service_protocol::log_collector_server::LogCollector
    for IndexLogCollectorServer
{
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

        // TODO decorrelate sending logs to quickwit from request handler (handler should never fail)

        match async {
            Ok::<QuickwitIngestResponse, Status>(
                self.http_client
                    .post(self.ingest_url.clone())
                    .json(&log_entry)
                    .send()
                    .map_err(|e| Status::unavailable(e.to_string()))
                    .await?
                    .error_for_status()
                    .map_err(|e| Status::unavailable(e.to_string()))?
                    .json::<QuickwitIngestResponse>()
                    .await
                    .map_err(|e| Status::unavailable(e.to_string()))?,
            )
        }
        .await
        {
            Ok(_) => {
                // NOTE: quickwit will not throw errors of the submitted document is not valid in respect with the index schema
                tracing::debug!("Indexed {log_entry:#?}");
                COLLECTOR_OUTPUT_COUNT
                    .with_label_values(&[
                        OUTPUT_SYSTEM_QUICKWIT_LABEL_VALUE,
                        OUTPUT_STATUS_OK_LABEL_VALUE,
                    ])
                    .inc();
            }
            Err(e) => {
                tracing::error!("Unable to index logs to quickwit {e}");
                COLLECTOR_OUTPUT_COUNT
                    .with_label_values(&[
                        OUTPUT_SYSTEM_QUICKWIT_LABEL_VALUE,
                        OUTPUT_STATUS_ERROR_LABEL_VALUE,
                    ])
                    .inc();
                Err(e)?;
            }
        }

        Ok(tonic::Response::new(()))
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
                Ok(IndexLogEntry {
                    message,
                    timestamp_secs: timestamp.seconds as u64,
                    timestamp_nanos: (timestamp.seconds * 1_000_000 + timestamp.nanos as i64)
                        as u64,
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

                Ok(IndexLogEntry {
                    message,
                    timestamp_secs: timestamp.seconds as u64,
                    timestamp_nanos: (timestamp.seconds * 1_000_000 + timestamp.nanos as i64)
                        as u64,
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
