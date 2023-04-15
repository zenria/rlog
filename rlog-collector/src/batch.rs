use std::time::Duration;

use tokio::select;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio_util::sync::CancellationToken;

pub fn launch_batch_collector<T: Send + 'static>(
    max_wait_time: Duration,
    max_batch_size: usize,
    shutdown_token: CancellationToken,
) -> (Sender<T>, Receiver<Vec<T>>) {
    let (sender, mut receiver) = channel(100);

    let (batch_sender, batch_receiver) = channel(100);

    tokio::spawn(async move {
        let mut buffer = Vec::with_capacity(max_batch_size);

        loop {
            let max_wait = tokio::time::sleep(max_wait_time);
            select! {
                _ = shutdown_token.cancelled() => {
                    // send buffer before exit
                    send_buffer(&mut buffer, &batch_sender).await;
                    return;
                }
                _ = max_wait => {
                    // waited too long, send the buffer
                    send_buffer(&mut buffer, &batch_sender).await;
                }
                log_line =  receiver.recv() => {
                    match log_line {
                        Some(log_line) => {
                            buffer.push(log_line);
                            if buffer.len() == max_batch_size{
                                // batch completed!
                                send_buffer(&mut buffer, &batch_sender).await;
                            }
                        },
                        None => todo!(),
                    }

                }
            }
        }
    });

    (sender, batch_receiver)
}

async fn send_buffer<T>(buffer: &mut Vec<T>, batch_sender: &Sender<Vec<T>>) {
    if buffer.len() == 0 {
        return;
    }
    let batch = buffer.drain(..).collect::<Vec<_>>();
    // ignore send errors
    let _ = batch_sender.send(batch).await;
}
