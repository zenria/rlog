use std::collections::HashMap;

use serde::Serialize;
use syslog::{Facility, Severity};
use tokio::{io::AsyncWriteExt, net::TcpStream};

type StructuredData = HashMap<String, HashMap<String, String>>;

pub fn send_syslog(
    msg: &str,
    process: &str,
    hostname: &str,
    pid: u32,
    facility: Facility,
    severity: Severity,
    syslog_address: &str,
) {
    let formatter = syslog::Formatter5424 {
        facility,
        hostname: Some(hostname.into()),
        process: process.into(),
        pid,
    };
    let mut logger = syslog::udp(formatter, "127.0.0.1:25485", syslog_address).unwrap();
    match severity {
        Severity::LOG_ERR => logger.err((123, StructuredData::new(), msg)).unwrap(),
        Severity::LOG_WARNING => logger.warning((123, StructuredData::new(), msg)).unwrap(),
        Severity::LOG_INFO => logger.info((123, StructuredData::new(), msg)).unwrap(),
        _ => todo!("This is not implemented"),
    }
}

pub struct GelfLogger {
    stream: TcpStream,
}

impl GelfLogger {
    pub async fn new(addr: &str) -> anyhow::Result<Self> {
        Ok(Self {
            stream: TcpStream::connect(addr).await?,
        })
    }

    pub async fn send_log<'a>(&mut self, log: &GelfLog<'a>) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(&log)?;
        self.stream.write_all(&bytes).await?;
        self.stream.write_u8(0).await?;
        Ok(())
    }
}

#[derive(Serialize)]
pub struct GelfLog<'a> {
    pub short_message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub long_message: Option<&'a str>,
    /// same  numbering as syslog severity
    pub level: usize,
    pub service: &'a str,
    pub host: &'a str,
    // in seconds
    pub timestamp: f64,
    // this should be a map
    #[serde(flatten)]
    pub extra_fields: serde_json::Value,
}
