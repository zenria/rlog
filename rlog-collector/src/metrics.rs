use std::time::Duration;

use lazy_static::lazy_static;
use prometheus::{
    register_int_counter, register_int_counter_vec, register_int_gauge_vec, Encoder, IntCounter,
    IntCounterVec, IntGaugeVec, TextEncoder,
};

lazy_static! {
    pub static ref SHIPPER_QUEUE_COUNT: IntGaugeVec = register_int_gauge_vec!(
        "rlog_shipper_queue_count",
        "Number of elements buffered in queues",
        &["hostname", "queue_name"]
    )
    .unwrap();
    pub static ref SHIPPER_PROCESSED_COUNT: IntCounterVec = register_int_counter_vec!(
        "rlog_shipper_processed_count",
        "Number of elements buffered in queues",
        &["hostname", "queue_name"]
    )
    .unwrap();
    pub static ref SHIPPER_ERROR_COUNT: IntCounterVec = register_int_counter_vec!(
        "rlog_shipper_error_count",
        "Number of elements in error in queues",
        &["hostname", "queue_name"]
    )
    .unwrap();
    pub static ref COLLECTOR_INDEXED_COUNT: IntCounter = register_int_counter!(
        "rlog_collector_indexed_count",
        "Number of elements output to various systems",
    )
    .unwrap();
    pub static ref COLLECTOR_OUTPUT_COUNT: IntCounterVec = register_int_counter_vec!(
        "rlog_collector_output_request_count",
        "Number of output requests",
        &["system", "status"]
    )
    .unwrap();
}

pub const OUTPUT_STATUS_OK_LABEL_VALUE: &str = "ok";
pub const OUTPUT_STATUS_ERROR_LABEL_VALUE: &str = "error";
pub const OUTPUT_STATUS_TOO_MANY_REQUEST_LABEL_VALUE: &str = "toomany";
pub const OUTPUT_SYSTEM_QUICKWIT_LABEL_VALUE: &str = "quickwit";

/// Generate the content of /metrics prometheus metrics gathering endpoint.
///
pub fn generate_metrics() -> String {
    // Gather the metrics.
    let mut buffer = vec![];
    let encoder = TextEncoder::new();
    let metric_families = prometheus::gather();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

/// Launch async process collector at specified interval. It requires a running tokio runtime!
pub fn launch_async_process_collector(interval: Duration) {
    tokio::task::spawn(collect(interval));
}
#[cfg(all(target_os = "linux"))]
async fn collect(interval: Duration) {
    use prometheus::core::Collector;
    let process_collector = prometheus::process_collector::ProcessCollector::for_self();
    loop {
        tracing::debug!("Collecting process info");
        process_collector.collect();
        tokio::time::sleep(interval).await;
    }
}

#[cfg(all(not(target_os = "linux")))]
async fn collect(_: Duration) {
    tracing::warn!("Collecting process info not available on this platform");
}
