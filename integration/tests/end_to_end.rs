#[cfg(test)]
#[tokio::test]
async fn nominal_end_to_end() -> Result<(), Box<dyn std::error::Error>> {
    use axum::http::Uri;
    use integration::{
        quickwit_mock::MockQuickwitServer,
        test_utils::{self, GelfLog, GelfLogger},
    };
    use rlog_collector::{CollectorServerConfig, LogSystem};
    use rlog_common::utils::init_logging;
    use rlog_grpc::tonic::transport::{Channel, Server};
    use rlog_shipper::ServerConfig;
    use serde_json::json;
    use std::{
        str::FromStr,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use syslog::Severity;
    use tokio::time::timeout;

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
        syslog_udp_bind_address: shipper_syslog_bind.clone(),
        gelf_tcp_bind_address: shipper_gelf_bind.clone(),
    })
    .await?;

    tokio::time::sleep(Duration::from_secs(1)).await;

    // send some messages via syslog
    test_utils::send_syslog(
        "hello world",
        "my_app",
        "my_host",
        1234,
        syslog::Facility::LOG_LOCAL0,
        syslog::Severity::LOG_INFO,
        &shipper_syslog_bind,
    );
    test_utils::send_syslog(
        "hello world2",
        "my_app2",
        "my_host",
        12345,
        syslog::Facility::LOG_MAIL,
        syslog::Severity::LOG_ERR,
        &shipper_syslog_bind,
    );

    // also send some gelf stuff
    let mut gelf_logger = GelfLogger::new(&shipper_gelf_bind).await?;
    gelf_logger
        .send_log(&GelfLog {
            short_message: "hello gelf short message",
            long_message: None,
            level: Severity::LOG_INFO as usize,
            service: "my_java_old_stuff",
            host: "my_gelf_host",
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs_f64(),
            extra_fields: json!({}),
        })
        .await?;

    gelf_logger
        .send_log(&GelfLog {
            short_message: "hello gelf short message 2",
            long_message: Some("This is my long message and should replace short message"),
            level: Severity::LOG_ERR as usize,
            service: "my_java_old_stuff",
            host: "my_gelf_host",
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs_f64(),
            extra_fields: json!({
                "custom_field": "this is really custom!",
                "custom_int": 123456
            }),
        })
        .await?;

    tokio::time::sleep(Duration::from_secs(2)).await;

    // batch shall be sent now...
    let received = quickwit_server.get_received().await;

    assert_eq!(received.len(), 4, "We should have received 4 logs by now!");
    assert_eq!("hello world", received[0].message);
    assert_eq!("hello world2", received[1].message);
    assert_eq!("hello gelf short message", received[2].message);
    assert_eq!("hello gelf short message 2", received[3].message);

    assert_eq!("my_app", received[0].service_name);
    assert_eq!("my_app2", received[1].service_name);
    assert_eq!("my_java_old_stuff", received[2].service_name);
    assert_eq!("my_java_old_stuff", received[3].service_name);

    assert_eq!("my_host", received[0].hostname);
    assert_eq!("my_host", received[1].hostname);
    assert_eq!("my_gelf_host", received[2].hostname);
    assert_eq!("my_gelf_host", received[3].hostname);

    assert_eq!("INFO", received[0].severity_text);
    assert_eq!("ERROR", received[1].severity_text);
    assert_eq!("INFO", received[2].severity_text);
    assert_eq!("ERROR", received[3].severity_text);

    assert_eq!(LogSystem::Syslog, received[0].log_system);
    assert_eq!(LogSystem::Syslog, received[1].log_system);
    assert_eq!(LogSystem::Gelf, received[2].log_system);
    assert_eq!(LogSystem::Gelf, received[3].log_system);

    assert_eq!("local0", received[0].free_fields.get("facility").unwrap());
    assert_eq!("mail", received[1].free_fields.get("facility").unwrap());

    assert_eq!(
        1234,
        received[0]
            .free_fields
            .get("proc_pid")
            .unwrap()
            .as_i64()
            .unwrap()
    );
    assert_eq!(
        12345,
        received[1]
            .free_fields
            .get("proc_pid")
            .unwrap()
            .as_i64()
            .unwrap()
    );
    assert_eq!(0, received[2].free_fields.len());
    assert_eq!(
        "This is my long message and should replace short message",
        received[3].free_fields.get("long_message").unwrap()
    );
    assert_eq!(
        "this is really custom!",
        received[3].free_fields.get("custom_field").unwrap()
    );
    assert_eq!(
        123456,
        received[3]
            .free_fields
            .get("custom_int")
            .unwrap()
            .as_i64()
            .unwrap()
    );

    let shutdown = futures::future::join(collector.shutdown(), shipper.shutdown());
    timeout(Duration::from_secs(2), shutdown)
        .await
        // this must now happen
        .expect("Timed out while waiting for shutdown");

    Ok(())
}
