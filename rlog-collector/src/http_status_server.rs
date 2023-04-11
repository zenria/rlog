use std::net::SocketAddr;

use anyhow::Context;
use axum::{routing::get, Router};

use crate::metrics::generate_metrics;

pub fn launch_server(bind_address: &str) -> anyhow::Result<()> {
    let sock_addr = bind_address
        .parse::<SocketAddr>()
        .context("Invalid http status server bind address")?;

    tokio::spawn(async move {
        let app = Router::new()
            .route("/health", get(|| async { "OK" }))
            .route("/metrics", get(|| async { generate_metrics() }));
        tracing::info!("Starting HTTP status server {sock_addr}");
        axum::Server::bind(&sock_addr)
            .serve(app.into_make_service())
            .await
            .unwrap();
    });

    Ok(())
}
