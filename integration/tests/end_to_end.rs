use std::io::Write;

#[cfg(test)]
#[tokio::test]
async fn nominal_end_to_end() -> Result<(), Box<dyn std::error::Error>> {
    use integration::test_utils::{self, BindAddresses, GelfLog};
    use regex::Regex;
    use rlog_collector::LogSystem;
    use rlog_common::utils::init_logging;
    use rlog_shipper::config::{FieldMapping, FieldType, FileParseConfig};
    use serde_json::json;
    use std::{
        collections::HashMap,
        sync::Arc,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };
    use syslog::Severity;
    use tempfile::NamedTempFile;
    use tokio::time::timeout;

    init_logging();

    // setup file watching config
    let mut tmp_file = NamedTempFile::new()?;

    let mut files_in = HashMap::new();
    files_in.insert(
        tmp_file.path().to_string_lossy().to_string(),
        FileParseConfig::Regex {
            pattern: Regex::new(r#"^\[([^\]]+)\]\[([^\]]+) *\]\[([^\]]+)\] \[([^\]]+)\] (.*)$"#)
                .unwrap(),

            mapping: vec![
                FieldMapping {
                    name: "timestamp".into(),
                    field_type: FieldType::Timestamp,
                },
                FieldMapping {
                    name: "level".into(),
                    field_type: FieldType::SyslogLevelText,
                },
                FieldMapping {
                    name: "_logger".into(),
                    field_type: FieldType::String,
                },
                FieldMapping {
                    name: "host".into(),
                    field_type: FieldType::String,
                },
                FieldMapping {
                    name: "short_message".into(),
                    field_type: FieldType::String,
                },
            ],
        },
    );

    rlog_shipper::config::CONFIG.store(Arc::new(rlog_shipper::config::Config {
        files_in,
        ..Default::default()
    }));
    tmp_file.write_all(
        format!("[2023-02-13T08:42:54,879][INFO ][o.e.n.Node               ] [sug6-dev-1] stopped")
            .as_bytes(),
    )?;
    tmp_file.write_all(
        format!("[2023-02-13T08:46:48,927][INFO ][o.e.e.NodeEnvironment    ] [sug6-dev-1] using [1] data paths, mounts [[/ (/dev/md1p1)]], net usable_space [669.4gb], net total_space [936gb], types [ext4]")
            .as_bytes(),
    )?;

    let bind_addresses = BindAddresses::default();

    let quickwit_server = bind_addresses.start_quickwit("rlog");
    let collector = bind_addresses.start_collector("rlog")?;
    let shipper = bind_addresses.start_shipper().await?;

    tokio::time::sleep(Duration::from_secs(1)).await;

    // send some messages via syslog
    test_utils::send_syslog(
        "hello world",
        "my_app",
        "my_host",
        1234,
        syslog::Facility::LOG_LOCAL0,
        syslog::Severity::LOG_INFO,
        &bind_addresses,
    );
    test_utils::send_syslog(
        "hello world2",
        "my_app2",
        "my_host",
        12345,
        syslog::Facility::LOG_MAIL,
        syslog::Severity::LOG_ERR,
        &bind_addresses,
    );

    tokio::time::sleep(Duration::from_millis(200)).await;

    // also send some gelf stuff
    let mut gelf_logger = bind_addresses.gelf_logger().await?;
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

    // also send a log with another gelf logger (this checks that our server accepts more than 1 connection)
    bind_addresses
        .gelf_logger()
        .await?
        .send_log(&GelfLog {
            short_message: "foobar :)",
            long_message: None,
            level: Severity::LOG_INFO as usize,
            service: "my_java_new_stuff",
            host: "my_gelf_host",
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs_f64(),
            extra_fields: json!({}),
        })
        .await?;

    tokio::time::sleep(Duration::from_millis(200)).await;

    // write to the file ; 1 log only
    tmp_file.write_all(format!("[2023-02-13T08:46:53,195][INFO ][o.e.p.PluginsService     ] [sug6-dev-1] loaded module [analysis-common]").as_bytes())?;

    tokio::time::sleep(Duration::from_secs(2)).await;

    // batch shall be sent now...
    let received = quickwit_server.get_received().await;

    assert_eq!(received.len(), 6, "We should have received 6 logs by now!");
    assert_eq!("hello world", received[0].message);
    assert_eq!("hello world2", received[1].message);
    assert_eq!("hello gelf short message", received[2].message);
    assert_eq!("hello gelf short message 2", received[3].message);
    assert_eq!("foobar :)", received[4].message);

    assert_eq!("my_app", received[0].service_name);
    assert_eq!("my_app2", received[1].service_name);
    assert_eq!("my_java_old_stuff", received[2].service_name);
    assert_eq!("my_java_old_stuff", received[3].service_name);
    assert_eq!("my_java_new_stuff", received[4].service_name);

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
