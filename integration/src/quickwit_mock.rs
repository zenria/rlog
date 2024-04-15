use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::State,
    routing::{get, post},
    Router,
};
use rlog_collector::IndexLogEntry;
use tokio::{net::TcpListener, sync::RwLock};

use crate::test_utils::BindAddresses;

/// Mock quickwit server

pub struct MockQuickwitServer {
    received: Arc<RwLock<Vec<IndexLogEntry>>>,
}

impl MockQuickwitServer {
    pub fn start(index_id: &str, bind_addresses: &BindAddresses) -> Self {
        let received = Arc::new(RwLock::new(vec![]));

        let ingest_route = format!("/api/v1/{index_id}/ingest");
        let app = Router::new()
            .route("/", get(|| async { "hello!" }))
            .route(
                &ingest_route,
                post(
                    |received: State<Arc<RwLock<Vec<IndexLogEntry>>>>, body: String| async move {
                        tracing::info!("Received: {body}");

                        let mut received = received.write().await;

                        for log in body.lines() {
                            match serde_json::from_str::<IndexLogEntry>(log) {
                                Ok(log_entry) => received.push(log_entry),
                                Err(e) => {
                                    tracing::error!("Unable to parse log entry -- {e} -- {log}")
                                }
                            }
                        }

                        "TODO: a real quickwit response"
                    },
                ),
            )
            .with_state(received.clone());
        let sock_addr = bind_addresses
            .quickwit_bind_address
            .parse::<SocketAddr>()
            .expect("Invalid http status server bind address");
        tokio::spawn(async move {
            axum::serve(
                TcpListener::bind(&sock_addr).await.unwrap(),
                app.into_make_service(),
            )
            .await
            .unwrap();
        });
        Self { received }
    }

    pub async fn get_received(&self) -> Vec<IndexLogEntry> {
        self.received.read().await.iter().cloned().collect()
    }

    pub fn url(bind_addresses: &BindAddresses) -> String {
        format!("http://{}/", bind_addresses.quickwit_bind_address)
    }
}
