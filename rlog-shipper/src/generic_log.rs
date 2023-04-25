//! minimal log requirements
//!
//!

use std::collections::HashMap;

use chrono::Utc;
use rlog_grpc::rlog_service_protocol::{LogLine, SyslogSeverity};
pub struct GenericLog {
    pub host: String,
    pub timestamp: chrono::DateTime<Utc>,
    pub severity: SyslogSeverity,
    pub extra: serde_json::Value,
    pub log_system: String,
    pub message: String,
    pub service_name: String,
}

impl TryFrom<GenericLog> for LogLine {
    type Error = anyhow::Error;

    fn try_from(value: GenericLog) -> Result<Self, Self::Error> {
        let timestamp = rlog_grpc::prost_wkt_types::Timestamp {
            seconds: value.timestamp.timestamp(),
            nanos: value.timestamp.timestamp_subsec_nanos() as i32,
        };

        let mut extra = HashMap::new();
        for (key, value) in value
            .extra
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("{} is not an object!", value.extra))?
        {
            let key = if key.starts_with('_') {
                &key[1..]
            } else {
                key.as_str()
            };
            extra.insert(key, value);
        }
        let extra = serde_json::to_string(&extra)?; // this cannot fail

        Ok(LogLine {
            host: value.host,
            timestamp: Some(timestamp),
            line: Some(
                rlog_grpc::rlog_service_protocol::log_line::Line::GenericLog(
                    rlog_grpc::rlog_service_protocol::GenericLogLine {
                        message: value.message,
                        severity: value.severity as i32,
                        service_name: value.service_name,
                        log_system: value.log_system,
                        extra,
                    },
                ),
            ),
        })
    }
}
