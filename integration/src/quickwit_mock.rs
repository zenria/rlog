use std::{net::SocketAddr, sync::Arc};

use axum::{
    extract::State,
    routing::{get, post},
    Router,
};
use rlog_collector::IndexLogEntry;
use tokio::sync::RwLock;

/// Mock quickwit server

pub struct MockQuickwitServer {
    received: Arc<RwLock<Vec<IndexLogEntry>>>,
    port: u16,
}

impl MockQuickwitServer {
    pub fn start(index_id: &str) -> Self {
        let port = portpicker::pick_unused_port().expect("Unable to find a free port");

        let received = Arc::new(RwLock::new(vec![]));

        let ingest_route = format!("/api/v1/{index_id}/ingest");
        let app = Router::new()
            .route("/", get(|| async { "hello!" }))
            .route(
                &ingest_route,
                post(
                    |received: State<Arc<RwLock<Vec<IndexLogEntry>>>>, body: String| async move {
                        tracing::debug!("Received: {body}");

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
        let sock_addr = format!("127.0.0.1:{port}")
            .parse::<SocketAddr>()
            .expect("Invalid http status server bind address");
        tokio::spawn(async move {
            axum::Server::bind(&sock_addr)
                .serve(app.into_make_service())
                .await
                .unwrap();
        });
        Self { received, port }
    }

    pub async fn get_received(&self) -> Vec<IndexLogEntry> {
        self.received.read().await.iter().cloned().collect()
    }

    pub fn url(&self) -> String {
        format!("https://localhost:{}/", self.port)
    }
}
