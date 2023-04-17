use arc_swap::ArcSwap;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

lazy_static! {
    pub static ref CONFIG: ArcSwap<Config> = ArcSwap::new(Arc::new(Config::default()));
}

#[derive(Serialize, Deserialize)]
pub struct Config {
    /// Size of the input buffer queue size (queue used before batch aggregation)
    pub collector_input_buffer_size: usize,
    /// Size of quickwit output buffer size (queue that contains batches)
    pub collector_quickwit_output_buffer_size: usize,
    /// Size of each log batch sent to quickwit
    pub collector_quickwit_batch_size: usize,
    /// Maximum interval between each batches: if there is not enough logs to
    /// emit a full batch a partial batch will be emitted if the last batch was
    /// emitted before this time
    #[serde(with = "humantime_serde")]
    pub collector_quickwit_batch_max_interval: Duration,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            collector_input_buffer_size: 1000,
            collector_quickwit_output_buffer_size: 100,
            collector_quickwit_batch_size: 100,
            collector_quickwit_batch_max_interval: Duration::from_secs(1),
        }
    }
}
