use std::{
    sync::atomic::AtomicU64,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use integration::test_utils::{BindAddresses, GelfLog};
use rand::Rng;
use rlog_common::utils::init_logging;
use serde_json::json;
use syslog::Severity;
use tokio::time::timeout;

#[tokio::test]
async fn many_shippers() -> anyhow::Result<()> {
    init_logging();
    let mut bind_addresses = BindAddresses::default();

    let quickwit = bind_addresses.start_quickwit("rlog");
    let collector = bind_addresses.start_collector("rlog")?;

    let mut shippers = vec![];

    let counter: &'static AtomicU64 = Box::leak(Box::new(AtomicU64::new(0)));

    for _ in 0..100 {
        let ba = bind_addresses.new_shipper_addresses();
        shippers.push(tokio::spawn(async move {
            let shipper = ba.start_shipper().await?;
            tokio::time::sleep(Duration::from_secs(2)).await;

            // send a gelf log
            let mut logger = ba.gelf_logger().await?;
            for _ in 0..50 {
                let count = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                logger
                    .send_log(&GelfLog {
                        short_message: &format!("hello {count}"),
                        long_message: None,
                        level: Severity::LOG_INFO as usize,
                        service: &format!("svc_{count}"),
                        host: &format!("host_{count}"),
                        timestamp: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_secs_f64(),
                        extra_fields: json!({}),
                    })
                    .await?;
                // wait between 10us to 10ms
                let wait_us = rand::thread_rng().gen_range(10..10000);
                tokio::time::sleep(Duration::from_micros(wait_us)).await;
            }
            drop(logger);
            // let a bit time for logs to be shipped to the collector
            //tokio::time::sleep(Duration::from_secs(1)).await;

            timeout(Duration::from_secs(120), shipper.shutdown()).await?;

            Ok::<_, anyhow::Error>(())
        }));
    }
    let results = futures::future::join_all(shippers).await;
    for r in results {
        r??;
    }

    tracing::info!("All shippers have shipped!");

    // let time for batches to be shipped
    tokio::time::sleep(Duration::from_secs(1)).await;

    timeout(Duration::from_secs(120), collector.shutdown()).await?;

    let received = quickwit.get_received().await;

    assert_eq!(
        counter.load(std::sync::atomic::Ordering::Relaxed),
        received.len() as u64
    );

    Ok(())
}
