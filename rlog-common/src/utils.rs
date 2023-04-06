use std::path::Path;

use anyhow::Context;
use tracing::metadata::LevelFilter;
use tracing_subscriber::{fmt::SubscriberBuilder, util::SubscriberInitExt, EnvFilter};

/// Read a file.
///
/// In case of error, a human readable context is added to the underlying error.
pub fn read_file<P: AsRef<Path>>(path: P) -> anyhow::Result<Vec<u8>> {
    let path = path.as_ref();
    std::fs::read(path).with_context(|| format!("Cannot open file {}", path.to_string_lossy()))
}

pub fn init_logging() {
    SubscriberBuilder::default()
        // only enable colored output on real terminals
        .with_ansi(atty::is(atty::Stream::Stdout))
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .finish()
        .init();
}
