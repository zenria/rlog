use std::{
    collections::BTreeMap,
    net::SocketAddr,
    time::{Duration, Instant},
};

use anyhow::Context;
use axum::{routing::get, Router};
use lazy_static::lazy_static;
use tokio::sync::RwLock;

use crate::metrics::generate_metrics;

pub const VERSION: &'static str = env!("CARGO_PKG_VERSION");

lazy_static! {
    static ref CONNECTED_SHIPPERS: RwLock<BTreeMap<String, Instant>> = RwLock::new(BTreeMap::new());
}

pub async fn report_connected_host(hostname: &str) {
    let mut shippers = CONNECTED_SHIPPERS.write().await;
    shippers.insert(hostname.into(), Instant::now());
}

async fn clear_disconnected_hosts() {
    let mut shippers = CONNECTED_SHIPPERS.write().await;
    let mut disconnected = Vec::new();
    let now = Instant::now();
    for (host, last_seen) in shippers.iter() {
        // shipper reports metrics every 30s, 90s should  be a very safe default
        if now.duration_since(last_seen.clone()) > Duration::from_secs(90) {
            disconnected.push(host.clone());
        }
    }
    for disconnected in disconnected {
        shippers.remove(&disconnected);
    }
}

pub fn launch_server(bind_address: &str) -> anyhow::Result<()> {
    tokio::spawn(async {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            clear_disconnected_hosts().await;
        }
    });

    let sock_addr = bind_address
        .parse::<SocketAddr>()
        .context("Invalid http status server bind address")?;

    tokio::spawn(async move {
        let app = Router::new()
            .route("/version", get(|| async { VERSION }))
            .route("/health", get(|| async { "OK" }))
            .route(
                "/connected-shippers",
                get(|| async {
                    let mut ret = String::new();
                    let shippers = CONNECTED_SHIPPERS.read().await;
                    for hostname in shippers.keys() {
                        ret.push_str(hostname);
                        ret.push('\n');
                    }
                    ret
                }),
            )
            .route("/metrics", get(|| async { generate_metrics() }));
        tracing::info!("Starting HTTP status server {sock_addr}");
        axum::Server::bind(&sock_addr)
            .serve(app.into_make_service())
            .await
            .unwrap();
    });

    Ok(())
}
