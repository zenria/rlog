use std::{fmt::Display, sync::atomic::Ordering};

use anyhow::{anyhow, Context};
use arc_swap::access::Access;
use async_channel::{Receiver, TrySendError};
use futures::FutureExt;
use rlog_grpc::rlog_service_protocol::{
    log_line::Line, LogLine, SyslogFacility, SyslogLogLine, SyslogSeverity,
};
use syslog_loose::Message;
use tokio::{net::UdpSocket, select};
use tokio_util::sync::CancellationToken;

use crate::{
    config::{Config, SyslogInputConfig, CONFIG},
    metrics::{SYSLOG_ERROR_COUNT, SYSLOG_QUEUE_COUNT},
};

pub struct SyslogLog(Message<String>);

impl Display for SyslogLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

pub async fn launch_syslog_udp_server(
    bind_address: &str,
    shutdown_token: CancellationToken,
) -> anyhow::Result<Receiver<SyslogLog>> {
    let config = CONFIG.map(|config: &Config| &config.syslog_in);
    let (sender, receiver) = async_channel::bounded(match config.load().as_ref() {
        Some(config) => config.common.max_buffer_size,
        None => SyslogInputConfig::default().common.max_buffer_size,
    });

    let socket = UdpSocket::bind(&bind_address)
        .await
        .context("Unable to listen to syslog UDP bind address")?;

    tracing::info!("Syslog server listening UDP {bind_address}");

    tokio::spawn(
        async move {
            // An udp packet cannot be larger than 65507 bytes.
            // Note: RFC 5424 requires the receiver should be able to handle
            // a minimum of 2048 bytes but we can afford to handle a bit more
            // bytes ;)
            let mut buf = [0u8; 65507];
            loop {
                select! {
                    _ = shutdown_token.cancelled() => {
                        return;
                    }
                    res = socket.recv_from(&mut buf) => {
                        let (n, from) = match res {
                            Ok(r) => r,
                            Err(e) => {
                                // this is highly unlikely!
                                tracing::error!("Unable to read UDP socket {e}");
                                continue;
                            }
                        };
                        let from = from.to_string();
                        let span = tracing::info_span!("syslog_in", remote_addr = from);
                        let _entered = span.enter();

                        let datagram = &buf[0..n];
                        let message = String::from_utf8_lossy(datagram);
                        tracing::debug!("Received {}", message);
                        let message = syslog_loose::parse_message(&message);

                        if filters::is_excluded(&message) {
                            continue;
                        }

                        let message: Message<String> = message.into();
                        tracing::debug!("Decoded {}", message);

                        if let Err(e) = sender.try_send(SyslogLog(message)) {
                            SYSLOG_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
                            match e {
                                TrySendError::Full(value) => {
                                    tracing::error!("Send buffer full: discarding value {}", value);
                                }
                                TrySendError::Closed(value) => {
                                    // this is not possible by construction...
                                    tracing::error!("Channel closed, discarding value {}", value);
                                }
                            }
                            return;
                        } else {
                            SYSLOG_QUEUE_COUNT.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            }
        }
        .then(|_| async { tracing::info!("Syslog server stopped.") }),
    );

    Ok(receiver)
}

mod filters {
    use syslog_loose::Message;

    use crate::config::{SyslogExclusionFilter, CONFIG};

    pub(super) fn is_excluded<T: AsRef<str> + Ord + PartialEq + Clone>(
        message: &Message<T>,
    ) -> bool {
        static EMPTY_FILTERS: Vec<SyslogExclusionFilter> = vec![];
        let config = CONFIG.load();
        let filters = match config.syslog_in.as_ref() {
            Some(config) => &config.exclusion_filters,
            None => &EMPTY_FILTERS,
        };
        for exclusion_filter in filters {
            // message will be excluded only if shall_exclude==Some(true)
            let mut shall_exclude = None;
            if let (Some(pattern), Some(appname)) = (&exclusion_filter.appname, &message.appname) {
                shall_exclude = Some(pattern.is_match(appname.as_ref()))
            }
            if let (Some(pattern), Some(facility)) = (&exclusion_filter.facility, message.facility)
            {
                shall_exclude = shall_exclude
                    // previous filter has been applied, it will be excluded only if this filter applies
                    .and_then(|excl| Some(excl && pattern.is_match(facility.as_str())))
                    // no previous filter in the config!
                    .or_else(|| Some(pattern.is_match(facility.as_str())));
            }
            if let Some(pattern) = &exclusion_filter.message {
                shall_exclude = shall_exclude
                    // previous filter has been applied, it will be excluded only if this filter applies
                    .and_then(|excl| Some(excl && pattern.is_match(message.msg.as_ref())))
                    // no previous filter in the config!
                    .or_else(|| Some(pattern.is_match(message.msg.as_ref())));
            }

            return shall_exclude.unwrap_or(false);
        }
        false
    }

    #[test]
    #[cfg(test)]
    fn test_excluded() {
        use std::{sync::Arc, vec};

        use crate::config::{eqregex::EqRegex, Config, SyslogExclusionFilter, SyslogInputConfig};

        let message = Message {
            protocol: syslog_loose::Protocol::RFC5424(0),
            facility: Some(syslog_loose::SyslogFacility::LOG_AUDIT),
            severity: None,
            timestamp: None,
            hostname: None,
            appname: Some("my-ultimate-app.sh"),
            procid: None,
            msgid: None,
            structured_data: vec![],
            msg: "natty line some stuff in there",
        };

        let message2 = Message {
            protocol: syslog_loose::Protocol::RFC5424(0),
            facility: Some(syslog_loose::SyslogFacility::LOG_AUDIT),
            severity: None,
            timestamp: None,
            hostname: None,
            appname: Some("postfix"),
            procid: None,
            msgid: None,
            structured_data: vec![],
            msg: "natty line some stuff in there",
        };

        assert!(!is_excluded(&message));

        let new_config = Config {
            syslog_in: Some(SyslogInputConfig {
                exclusion_filters: vec![SyslogExclusionFilter {
                    appname: Some(EqRegex::new("my-ultimate-app.*").unwrap()),
                    facility: None,
                    message: Some(EqRegex::new("natty").unwrap()),
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        CONFIG.store(Arc::new(new_config));

        assert!(is_excluded(&message));
        assert!(!is_excluded(&message2));
    }
}

impl TryFrom<SyslogLog> for LogLine {
    type Error = anyhow::Error;

    fn try_from(value: SyslogLog) -> Result<Self, Self::Error> {
        let value = value.0;
        let hostname = value
            .hostname
            .ok_or(anyhow::anyhow!("No hostname in syslog"))?;

        let timestamp = value.timestamp.ok_or(anyhow!("No timestamp in syslog"))?;

        let timestamp_secs = timestamp.timestamp();
        let nanos = timestamp.timestamp_subsec_nanos();

        let message = value.msg;

        let severity = value.severity.ok_or(anyhow!("No severity in syslog"))?;

        let (proc_pid, proc_name) = value
            .procid
            .map(|procid| match procid {
                syslog_loose::ProcId::PID(pid) => (Some(pid), None),
                syslog_loose::ProcId::Name(name) => (None, Some(name)),
            })
            .unwrap_or((None, None));

        Ok(LogLine {
            host: hostname,
            timestamp: Some(rlog_grpc::prost_wkt_types::Timestamp {
                seconds: timestamp_secs,
                nanos: nanos as i32,
            }),
            line: Some(Line::Syslog(SyslogLogLine {
                facility: value
                    .facility
                    .map(to_grpc_facility)
                    .unwrap_or(SyslogFacility::Local0) as i32,
                severity: to_grpc_severity(severity) as i32,
                appname: value.appname,
                proc_pid,
                proc_name,
                msgid: value.msgid,
                msg: message,
            })),
        })
    }
}

fn to_grpc_facility(facility: syslog_loose::SyslogFacility) -> SyslogFacility {
    use SyslogFacility::*;
    match facility {
        syslog_loose::SyslogFacility::LOG_KERN => Kernel,
        syslog_loose::SyslogFacility::LOG_USER => User,
        syslog_loose::SyslogFacility::LOG_MAIL => Mail,
        syslog_loose::SyslogFacility::LOG_DAEMON => Daemon,
        syslog_loose::SyslogFacility::LOG_AUTH => Auth,
        syslog_loose::SyslogFacility::LOG_SYSLOG => Syslog,
        syslog_loose::SyslogFacility::LOG_LPR => Lpr,
        syslog_loose::SyslogFacility::LOG_NEWS => News,
        syslog_loose::SyslogFacility::LOG_UUCP => Uucp,
        syslog_loose::SyslogFacility::LOG_CRON => Cron,
        syslog_loose::SyslogFacility::LOG_AUTHPRIV => Authpriv,
        syslog_loose::SyslogFacility::LOG_FTP => Ftp,
        syslog_loose::SyslogFacility::LOG_NTP => Ntp,
        syslog_loose::SyslogFacility::LOG_AUDIT => Audit,
        syslog_loose::SyslogFacility::LOG_ALERT => Alert,
        syslog_loose::SyslogFacility::LOG_CLOCKD => Clockd,
        syslog_loose::SyslogFacility::LOG_LOCAL0 => Local0,
        syslog_loose::SyslogFacility::LOG_LOCAL1 => Local1,
        syslog_loose::SyslogFacility::LOG_LOCAL2 => Local2,
        syslog_loose::SyslogFacility::LOG_LOCAL3 => Local3,
        syslog_loose::SyslogFacility::LOG_LOCAL4 => Local4,
        syslog_loose::SyslogFacility::LOG_LOCAL5 => Local5,
        syslog_loose::SyslogFacility::LOG_LOCAL6 => Local6,
        syslog_loose::SyslogFacility::LOG_LOCAL7 => Local7,
    }
}

fn to_grpc_severity(severity: syslog_loose::SyslogSeverity) -> SyslogSeverity {
    use SyslogSeverity::*;
    match severity {
        syslog_loose::SyslogSeverity::SEV_EMERG => Emergency,
        syslog_loose::SyslogSeverity::SEV_ALERT => Alert,
        syslog_loose::SyslogSeverity::SEV_CRIT => Critical,
        syslog_loose::SyslogSeverity::SEV_ERR => Error,
        syslog_loose::SyslogSeverity::SEV_WARNING => Warning,
        syslog_loose::SyslogSeverity::SEV_NOTICE => Notice,
        syslog_loose::SyslogSeverity::SEV_INFO => Info,
        syslog_loose::SyslogSeverity::SEV_DEBUG => Debug,
    }
}
