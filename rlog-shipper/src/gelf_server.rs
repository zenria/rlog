use anyhow::Context;
use bytes::BytesMut;
use rlog_grpc::rlog_service_protocol::GelfLogLine;
use serde_json::Value;
use tokio::{
    io::AsyncReadExt,
    net::TcpListener,
    sync::mpsc::{channel, Receiver},
};

pub async fn launch_gelf_server(bind_address: &str) -> anyhow::Result<Receiver<GelfLogLine>> {
    let (sender, receiver) = channel(10000);

    let listener = TcpListener::bind(bind_address)
        .await
        .context("Unable to bind to GELF bind address")?;

    println!("Launching GELF TCP server at {bind_address}");

    loop {
        let (mut socket, r) = listener.accept().await?;
        println!("Connected from {r}");
        tokio::spawn(async move {
            let mut buffer = BytesMut::with_capacity(4096);
            // In a loop, read data from the socket and write the data back.
            loop {
                let n = match socket.read_buf(&mut buffer).await {
                    // graceful shutdown
                    Ok(n) if n == 0 && buffer.len() == 0 => break,
                    // connection closed during transmission of a frame
                    Ok(n) if n == 0 => {
                        eprintln!("Connection reset by peer");
                        break;
                    }
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("failed to read from socket; err = {:?}", e);
                        return;
                    }
                };
                // check we received a \0 bytes indicating the end of a frame
                while let Some(i) = buffer
                    .iter()
                    .enumerate()
                    .find(|(_i, byte)| byte == &&0)
                    .map(|(i, _)| i)
                {
                    // there is a message between cursor & i
                    match serde_json::from_slice::<Value>(&buffer[0..i]) {
                        Ok(valid_json) => println!("GELF got: {valid_json}"),
                        Err(e) => eprint!("Unable to decode json..."),
                    }
                    // remove the first part of the buffer and discard it
                    let _ = buffer.split_to(i + 1);
                }
            }
            println!("Connection from {r} closed.");
        });
    }

    Ok(receiver)
}
