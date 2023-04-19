use std::{collections::HashMap, str::FromStr};

use axum::http::Uri;
use rlog_collector::{CollectorServer, CollectorServerConfig};
use rlog_grpc::tonic::transport::{Channel, Server};
use rlog_shipper::{ServerConfig, ShipperServer};
use serde::Serialize;
use syslog::{Facility, Severity};
use tokio::{io::AsyncWriteExt, net::TcpStream};

use crate::quickwit_mock::MockQuickwitServer;

type StructuredData = HashMap<String, HashMap<String, String>>;

pub fn send_syslog(
    msg: &str,
    process: &str,
    hostname: &str,
    pid: u32,
    facility: Facility,
    severity: Severity,
    addresses: &BindAddresses,
) {
    let formatter = syslog::Formatter5424 {
        facility,
        hostname: Some(hostname.into()),
        process: process.into(),
        pid,
    };
    let mut logger =
        syslog::udp(formatter, "127.0.0.1:25485", &addresses.shipper_syslog_bind).unwrap();
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

fn find_open_ports<const N: usize>() -> [u16; N] {
    let mut ret = [0u16; N];

    for i in 0..N {
        let mut candidate = portpicker::pick_unused_port().expect("Unable to pick unused port");
        while ret[..i].contains(&candidate) {
            candidate = portpicker::pick_unused_port().expect("Unable to pick unused port");
        }
        ret[i] = candidate;
    }

    ret
}

pub struct BindAddresses {
    pub grpc_bind_address: String,
    pub shipper_gelf_bind: String,
    pub shipper_syslog_bind: String,
    pub collector_http_bind: String,
    pub quickwit_bind_address: String,
}
impl Default for BindAddresses {
    fn default() -> Self {
        let ports = find_open_ports::<5>();
        Self {
            grpc_bind_address: format!("127.0.0.1:{}", ports[0]),
            shipper_gelf_bind: format!("127.0.0.1:{}", ports[1]),
            shipper_syslog_bind: format!("127.0.0.1:{}", ports[2]),
            collector_http_bind: format!("127.0.0.1:{}", ports[3]),
            quickwit_bind_address: format!("127.0.0.1:{}", ports[4]),
        }
    }
}

impl BindAddresses {
    pub fn start_quickwit(&self, index_id: &str) -> MockQuickwitServer {
        MockQuickwitServer::start(index_id, &self)
    }

    pub fn start_collector(&self, index_id: &str) -> Result<CollectorServer, anyhow::Error> {
        rlog_collector::CollectorServer::start_collector_server(CollectorServerConfig {
            http_status_bind_address: self.collector_http_bind.clone(),
            grpc_bind_address: self.grpc_bind_address.clone(),
            quickwit_rest_url: MockQuickwitServer::url(&self),
            quickwit_index_id: index_id.to_string(),
            server: Server::builder(),
        })
    }

    pub async fn start_shipper(&self) -> Result<ShipperServer, anyhow::Error> {
        rlog_shipper::ShipperServer::start_shipper_server(ServerConfig {
            grpc_collector_endpoint: Channel::builder(Uri::from_str(&format!(
                "http://{}",
                self.grpc_bind_address
            ))?),
            syslog_udp_bind_address: self.shipper_syslog_bind.clone(),
            gelf_tcp_bind_address: self.shipper_gelf_bind.clone(),
        })
        .await
    }

    /// This will try to connect to gelf in TCP so the shipper server
    /// must be started before starting this.
    pub async fn gelf_logger(&self) -> anyhow::Result<GelfLogger> {
        GelfLogger::new(&self.shipper_gelf_bind).await
    }
}
