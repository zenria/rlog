use sled::{IVec, Tree};
use tokio::sync::Notify;

struct Queue {
    db: Tree,
    new_data_notifier: Notify,
}

impl Queue {
    pub async fn recv_batch(
        &self,
        batch_max_size: usize,
        start: Option<IVec>,
    ) -> Vec<(Vec<IVec>, Vec<IVec>)> {
        let mut keys = Vec::new();
        let mut values = Vec::new();
        // iterate from "start" to the end
        let scan = self
            .db
            .range(start.unwrap_or(vec![].into())..)
            .take(batch_max_size);
        for r in scan {
            let (k, v) = r.unwrap();
            keys.push(k);
            values.push(v);
        }

        todo!()
    }
}
