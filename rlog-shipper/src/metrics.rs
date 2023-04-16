use std::{
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering::Relaxed},
};

use lazy_static::lazy_static;
use rlog_grpc::rlog_service_protocol::Metrics;

lazy_static! {
    pub static ref GELF_QUEUE_COUNT: AtomicU64 = AtomicU64::new(0);
    pub static ref SYSLOG_QUEUE_COUNT: AtomicU64 = AtomicU64::new(0);
    pub static ref SHIPPER_QUEUE_COUNT: AtomicU64 = AtomicU64::new(0);
    pub static ref GELF_PROCESSED_COUNT: AtomicU64 = AtomicU64::new(0);
    pub static ref SYSLOG_PROCESSED_COUNT: AtomicU64 = AtomicU64::new(0);
    pub static ref SHIPPER_PROCESSED_COUNT: AtomicU64 = AtomicU64::new(0);
    pub static ref SHIPPER_ERROR_COUNT: AtomicU64 = AtomicU64::new(0);
    pub static ref GELF_ERROR_COUNT: AtomicU64 = AtomicU64::new(0);
    pub static ref SYSLOG_ERROR_COUNT: AtomicU64 = AtomicU64::new(0);
}

pub(crate) fn to_grpc_metrics() -> Metrics {
    Metrics {
        hostname: hostname::get().unwrap().to_string_lossy().to_string(),
        queue_count: {
            let mut map = HashMap::new();
            map.insert("glef_in".into(), GELF_QUEUE_COUNT.load(Relaxed));
            map.insert("syslog_in".into(), SYSLOG_QUEUE_COUNT.load(Relaxed));
            map.insert("grpc_out".into(), SHIPPER_QUEUE_COUNT.load(Relaxed));
            map
        },
        processed_count: {
            let mut map = HashMap::new();
            map.insert("glef_in".into(), GELF_PROCESSED_COUNT.load(Relaxed));
            map.insert("syslog_in".into(), SYSLOG_PROCESSED_COUNT.load(Relaxed));
            map.insert("grpc_out".into(), SHIPPER_PROCESSED_COUNT.load(Relaxed));
            map
        },
        error_count: {
            let mut map = HashMap::new();
            map.insert("glef_in".into(), GELF_ERROR_COUNT.load(Relaxed));
            map.insert("syslog_in".into(), SYSLOG_ERROR_COUNT.load(Relaxed));
            map.insert("grpc_out".into(), SHIPPER_ERROR_COUNT.load(Relaxed));
            map
        },
    }
}
