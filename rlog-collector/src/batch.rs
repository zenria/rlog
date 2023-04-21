use std::time::Duration;

use arc_swap::access::Access;
use async_channel::{Receiver, SendError, Sender};
use tokio::select;
use tokio_util::sync::CancellationToken;

// working with arc-swapped config is rather extreme in term of generic stuff
// maybe this is a bit over-engineered!
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
    let (sender, receiver) = async_channel::bounded(*input_buffer_size.load());

    let (batch_sender, batch_receiver) = async_channel::bounded(*output_buffer_size.load());

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
                    while let Ok(item) = receiver.recv().await {
                        buffer.push(item);
                    }
                    // send buffer & exit
                    if let Err(_) = send_buffer(&mut buffer, &batch_sender).await{
                        tracing::error!("Batch channel closed!");
                    }
                    return;
                }
                _ = max_wait => {
                    // waited too long, send the buffer
                    if let Err(_) =  send_buffer(&mut buffer, &batch_sender).await{
                        tracing::error!("Batch channel closed!");
                    }
                }
                // we are responsible for channel closing ; by construction,
                // we must ignore recv() errors
                Ok(log_line) =  receiver.recv() => {
                    buffer.push(log_line);
                    if buffer.len() == *max_batch_size.load(){
                        // batch completed!
                        if let Err(_) =  send_buffer(&mut buffer, &batch_sender).await{
                            tracing::error!("Batch channel closed!");
                        }
                    }
                }
            }
        }
    });

    (sender, batch_receiver)
}

async fn send_buffer<T>(
    buffer: &mut Vec<T>,
    batch_sender: &Sender<Vec<T>>,
) -> Result<(), SendError<Vec<T>>> {
    if buffer.len() > 0 {
        let batch = buffer.drain(..).collect::<Vec<_>>();
        // ignore send errors
        batch_sender.send(batch).await
    } else {
        Ok(())
    }
}
