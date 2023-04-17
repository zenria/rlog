#[cfg(test)]
#[tokio::test]
async fn end_to_end() -> Result<(), Box<dyn std::error::Error>> {
    use axum::http::Uri;
    use integration::quickwit_mock::MockQuickwitServer;
    use rlog_collector::CollectorServerConfig;
    use rlog_common::utils::init_logging;
    use rlog_grpc::tonic::transport::{Channel, Server};
    use rlog_shipper::ServerConfig;
    use std::{str::FromStr, time::Duration};

    init_logging();

    let grpc_bind_address = format!(
        "127.0.0.1:{}",
        portpicker::pick_unused_port().expect("Cannot find an unused port")
    );
    let shipper_gelf_bind = format!(
        "127.0.0.1:{}",
        portpicker::pick_unused_port().expect("Cannot find an unused port")
    );
    let shipper_syslog_bind = format!(
        "127.0.0.1:{}",
        portpicker::pick_unused_port().expect("Cannot find an unused port")
    );
    let collector_http_bind = format!(
        "127.0.0.1:{}",
        portpicker::pick_unused_port().expect("Cannot find an unused port")
    );

    let quickwit_server = MockQuickwitServer::start("rlog");

    let collector =
        rlog_collector::CollectorServer::start_collector_server(CollectorServerConfig {
            http_status_bind_address: collector_http_bind,
            grpc_bind_address: grpc_bind_address.clone(),
            quickwit_rest_url: quickwit_server.url(),
            quickwit_index_id: "rlog".to_string(),
            server: Server::builder(),
        })?;

    let shipper = rlog_shipper::ShipperServer::start_shipper_server(ServerConfig {
        grpc_collector_endpoint: Channel::builder(Uri::from_str(&format!(
            "http://{grpc_bind_address}"
        ))?),
        syslog_udp_bind_address: shipper_syslog_bind,
        gelf_tcp_bind_address: shipper_gelf_bind,
    })
    .await?;

    tokio::time::sleep(Duration::from_secs(2)).await;

    collector.shutdown().await;
    shipper.shutdown().await;

    Ok(())
}
