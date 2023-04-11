use std::time::Duration;

use lazy_static::lazy_static;
use prometheus::{
    register_int_counter_vec, register_int_gauge_vec, Encoder, IntCounterVec, IntGaugeVec,
    TextEncoder,
};

lazy_static! {
    pub(crate) static ref SHIPPER_QUEUE_COUNT: IntGaugeVec = register_int_gauge_vec!(
        "rlog_shipper_queue_count",
        "Number of elements buffered in queues",
        &["hostname", "queue_name"]
    )
    .unwrap();
    pub(crate) static ref SHIPPER_PROCESSED_COUNT: IntCounterVec = register_int_counter_vec!(
        "rlog_shipper_processed_count",
        "Number of elements buffered in queues",
        &["hostname", "queue_name"]
    )
    .unwrap();
}

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
