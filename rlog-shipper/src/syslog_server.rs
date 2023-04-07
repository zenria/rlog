use anyhow::Context;
use syslog_loose::Message;
use tokio::{
    net::UdpSocket,
    sync::mpsc::{channel, error::TrySendError, Receiver},
};

pub async fn launch_syslog_udp_server(
    bind_address: &str,
) -> anyhow::Result<Receiver<syslog_loose::Message<String>>> {
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

            if let Err(e) = sender.try_send(message) {
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
            }
        }
    });

    Ok(receiver)
}
