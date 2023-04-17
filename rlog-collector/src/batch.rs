use std::time::Duration;

use arc_swap::access::Access;
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio_util::sync::CancellationToken;

// working with arc-swapped config is rather extreme in term of generic stuff
pub fn launch_batch_collector<T, D, S, IS, OS>(
    max_wait_time: D,
    max_batch_size: S,
    input_buffer_size: IS,
    output_buffer_size: OS,
    shutdown_token: CancellationToken,
) -> (Sender<T>, Receiver<Vec<T>>)
where
    T: Send + 'static,
    D: Access<Duration> + Send + 'static,
    S: Access<usize> + Send + 'static,
    IS: Access<usize> + Send + 'static,
    OS: Access<usize> + Send + 'static,
{
    let (sender, mut receiver) = channel(*input_buffer_size.load());

    let (batch_sender, batch_receiver) = channel(*output_buffer_size.load());

    tokio::spawn(async move {
        let mut buffer = Vec::with_capacity(*max_batch_size.load());

        loop {
            let max_wait = tokio::time::sleep(*max_wait_time.load());
            select! {
                _ = shutdown_token.cancelled() => {
                    // close the receiver: at this time, the grpc server
                    // will answer "unavailable" to all incoming requests
                    receiver.close();
                    // drain the receiver and put it for the last batch
                    while let Some(item) = receiver.recv().await {
                        buffer.push(item);
                    }
                    // send buffer & exit
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
                            if buffer.len() == *max_batch_size.load(){
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
