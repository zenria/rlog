use std::{fmt::Display, sync::atomic::Ordering};

use anyhow::{anyhow, Context};
use rlog_grpc::rlog_service_protocol::{
    log_line::Line, LogLine, SyslogFacility, SyslogLogLine, SyslogSeverity,
};
use syslog_loose::Message;
use tokio::{
    net::UdpSocket,
    sync::mpsc::{channel, error::TrySendError, Receiver},
};

use crate::metrics::SYSLOG_QUEUE_COUNT;

pub struct SyslogLog(Message<String>);

impl Display for SyslogLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

pub async fn launch_syslog_udp_server(bind_address: &str) -> anyhow::Result<Receiver<SyslogLog>> {
    let (sender, receiver) = channel(10_000);

    let socket = UdpSocket::bind(&bind_address)
        .await
        .context("Unable to listen to syslog UDP bind address")?;

    tracing::info!("Syslog server listening UDP {bind_address}");

    tokio::spawn(async move {
        // An udp packet cannot be larger than 65507 bytes.
        // Note: RFC 5424 requires the receiver should be able to handle
        // a minimum of 2048 bytes but we can afford to handle a bit more
        // bytes ;)
        let mut buf = [0u8; 65507];
        loop {
            let (n, from) = match socket.recv_from(&mut buf).await {
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
            let message: Message<String> = message.into();
            tracing::debug!("Decoded {}", message);

            if let Err(e) = sender.try_send(SyslogLog(message)) {
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
    });

    Ok(receiver)
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