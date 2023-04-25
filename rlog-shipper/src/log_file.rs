use anyhow::{anyhow, Context};
use async_channel::Receiver;
use chrono::prelude::*;
use chrono::{DateTime, FixedOffset};
use futures::FutureExt;
use lazy_static::lazy_static;
use linemux::MuxedLines;
use num_traits::FromPrimitive;
use rlog_common::utils::format_error;
use rlog_grpc::rlog_service_protocol::SyslogSeverity;
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::config::CONFIG;
use crate::config::{FieldType, FileParseConfig};
use crate::generic_log::GenericLog;

// Note: let's use the Gelf log repr which seems flexible enough ;)
pub async fn watch_log(
    path: &str,
    shutdown_token: CancellationToken,
) -> anyhow::Result<Receiver<GenericLog>> {
    // for now this is not configurable, we have only 1 buffer size
    let (sender, receiver) = async_channel::bounded(1);

    let path = path.to_owned();
    let file = path.clone(); // used in tracing span
    let mut lines = MuxedLines::new()?;
    lines.add_file(&path).await?;
    tracing::info!("Watching new lines of {path}");

    tokio::spawn(
        async move {
            select! {
                _ = shutdown_token.cancelled() => {
                    // shutting down
                    return;
                }
                line = lines.next_line() => {
                    match line {
                        Ok(line)=>{
                            match line {
                                Some(line)=> {
                                    tracing::debug!("new line {}", line.line());
                                    // find right config ; if config cannot be found, stop watching the file
                                    match CONFIG.load().files_in.get(&path){
                                        Some(parse_config) => {
                                            match parse_config.to_log(line.line(), &path) {
                                                Ok(log) => match sender.send(log).await {
                                                    Ok(_) => {},
                                                    Err(_closed) => tracing::error!("out channel closed"),
                                                },
                                                Err(e) => tracing::error!("Unable to parse file line {} - {}", line.line(), format_error(e)),
                                            }
                                        },
                                        None => {
                                            tracing::info!("Config changed: {path} is not monitored anymore!");
                                            return;
                                        },
                                    }
                                }
                                None=> {
                                    tracing::error!("This is not possible by contruction");
                                    return;
                                }
                            }

                        }
                        Err(e)=>{
                            tracing::error!("Unable to read log line! {e}");
                            return;
                        }
                    }
                }

            }
        }
        .then(|_| async  { tracing::info!("Watch task stopped!") })
        .instrument(tracing::info_span!("files_in", file)),
    );

    Ok(receiver)
}

lazy_static! {
    static ref HOSTNAME: String = hostname::get()
        .expect("Unable to get system hostname")
        .to_string_lossy()
        .to_string();
}

impl FileParseConfig {
    pub fn to_log(&self, line: &str, file: &str) -> anyhow::Result<GenericLog> {
        match self {
            FileParseConfig::Regex { pattern, mapping } => {
                let captures = pattern
                    .captures(line)
                    .ok_or_else(|| anyhow!("Not matching line: {line}"))?;

                let mut map = serde_json::Map::new();

                let mut host = None;
                let mut timestamp = None;
                let mut service_name = None;
                let mut severity = None;
                let mut message = None;

                for i in 0..mapping.len() {
                    let field_name = &mapping[i].name;
                    let field_value = captures
                        .get(i + 1)
                        .ok_or_else(|| anyhow!("missing capture group for field {field_name}"))?
                        .as_str()
                        .trim();
                    if field_name == "timestamp" {
                        timestamp = Some(parse_timestamp(field_value).with_context(|| {
                            anyhow!("Incorrect value for field {field_name}: {field_value}")
                        })?);
                        continue;
                    }
                    if field_name == "host" {
                        host = Some(field_value.to_string());
                        continue;
                    }
                    if field_name == "message" {
                        message = Some(field_value.to_string());
                        continue;
                    }
                    if field_name == "service_name" {
                        service_name = Some(field_value.to_string());
                        continue;
                    }
                    if field_name == "severity" {
                        severity = SyslogSeverity::from_str_name(field_value.trim());
                        continue;
                    }
                    let field_value = match &mapping[i].field_type {
                        FieldType::String => serde_json::Value::String(field_value.to_string()),
                        FieldType::Timestamp => serde_json::Value::String(
                            parse_timestamp(field_value)
                                .with_context(|| {
                                    anyhow!("Incorrect value for field {field_name}: {field_value}")
                                })?
                                .to_rfc3339(),
                        ),
                        FieldType::Number => {
                            serde_json::Value::Number(field_value.parse().with_context(|| {
                                format!("Incorrect value for field {field_name}: {field_value}")
                            })?)
                        }
                        FieldType::SyslogLevelText => serde_json::Value::Number(
                            (SyslogSeverity::from_str_name(field_value.trim())
                                .unwrap_or(SyslogSeverity::Info)
                                as u32)
                                .into(),
                        ),
                    };

                    map.insert(field_name.clone(), field_value);
                }

                Ok(GenericLog {
                    host: host.unwrap_or(HOSTNAME.to_string()),
                    timestamp: timestamp.unwrap_or_else(|| Utc::now()),
                    severity: severity.unwrap_or(SyslogSeverity::Info),
                    log_system: "file_in".into(),
                    message: message.ok_or_else(|| anyhow!("No message field defined!"))?,
                    extra: map.into(),
                    service_name: service_name.unwrap_or_else(|| file.to_string()),
                })
            }
        }
    }
}

fn parse_timestamp(ts: &str) -> anyhow::Result<DateTime<Utc>> {
    iso8601::datetime(ts)
        .map(|dt| {
            let tz = FixedOffset::east_opt(
                dt.time.tz_offset_hours.signum()
                    * (dt.time.tz_offset_hours.abs() * 60 + dt.time.tz_offset_minutes),
            )
            .ok_or_else(|| anyhow!("Invalid offset in timestamp {ts}"))?;

            let date = match dt.date {
                iso8601::Date::YMD { year, month, day } => {
                    NaiveDate::from_ymd_opt(year, month, day).ok_or_else(|| anyhow!("invalid date"))
                }
                iso8601::Date::Week { year, ww, d } => NaiveDate::from_isoywd_opt(
                    year,
                    ww,
                    Weekday::from_i64(d as i64).ok_or_else(|| anyhow!("dspj"))?,
                )
                .ok_or_else(|| anyhow!("invalid date")),
                iso8601::Date::Ordinal { year, ddd } => {
                    NaiveDate::from_yo_opt(year, ddd).ok_or_else(|| anyhow!("invalid date"))
                }
            }?;
            let time = NaiveTime::from_hms_milli_opt(
                dt.time.hour,
                dt.time.minute,
                dt.time.second,
                dt.time.millisecond,
            )
            .ok_or_else(|| anyhow!("invalid time"))?;

            tz.from_local_datetime(&NaiveDateTime::new(date, time))
                .earliest()
                .ok_or_else(|| anyhow!("invalid date"))
        })
        .map_err(|e| anyhow!({ e }))
        .and_then(|r| r)
        .or_else(|_| DateTime::parse_from_rfc3339(ts).context("Unable to parse date"))
        .or_else(|_| DateTime::parse_from_rfc2822(ts).context("Unable to parse date"))
        .map(|dt| dt.into())
}
