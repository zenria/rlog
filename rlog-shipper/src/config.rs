use std::{
    fs::File,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use anyhow::Context;
use arc_swap::ArcSwap;
use lazy_static::lazy_static;
use regex::Regex;
use rlog_common::utils::format_error;
use serde::Deserialize;

lazy_static! {
    pub static ref CONFIG: ArcSwap<Config> = ArcSwap::new(Arc::new(Config::default()));
}
// config will be check for modification every 10s
pub const CONFIG_REFRESH_INTERVAL: Duration = Duration::from_secs(10);

pub fn setup_config_from_file(path: &str) -> anyhow::Result<()> {
    let mut last_modified = load_and_swap_config(path)?;

    let path = path.to_string();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(CONFIG_REFRESH_INTERVAL).await;
            if let Some(modified) = std::fs::metadata(&path).and_then(|m| m.modified()).ok() {
                if modified != last_modified {
                    tracing::info!("Config file modified, reloading it!");
                    match load_and_swap_config(&path) {
                        Ok(m) => last_modified = m,
                        Err(e) => tracing::error!("Unable to reload config: {}", format_error(e)),
                    }
                }
            }
        }
    });

    Ok(())
}

fn load_and_swap_config<P: AsRef<Path>>(path: P) -> anyhow::Result<SystemTime> {
    let (config, last_modified) = load_config(path)?;

    CONFIG.swap(Arc::new(config));

    Ok(last_modified)
}

fn load_config<P: AsRef<Path>>(path: P) -> anyhow::Result<(Config, SystemTime)> {
    let file = File::open(path.as_ref()).with_context(|| {
        format!(
            "Cannot open config file at: {}",
            path.as_ref().to_string_lossy()
        )
    })?;

    let last_modified = file.metadata()?.modified()?;

    Ok((
        serde_yaml::from_reader(file).with_context(|| {
            format!(
                "Invalid YAML in config file at: {}",
                path.as_ref().to_string_lossy()
            )
        })?,
        last_modified,
    ))
}

#[derive(Deserialize, Default)]
pub struct Config {
    pub syslog_in: SyslogInputConfig,
    pub gelf_in: GelfInputConfig,
    pub grpc_out: GrpcOutConfig,
}

#[derive(Deserialize)]
pub struct GrpcOutConfig {
    pub max_buffer_size: usize,
}
impl Default for GrpcOutConfig {
    fn default() -> Self {
        Self {
            /// This will not be hot reloaded (buffer is allocated at the start of the application)
            max_buffer_size: 20_000,
        }
    }
}

#[derive(Deserialize)]
pub struct CommonInputConfig {
    /// This will not be hot reloaded (buffer is allocated at the start of the application)
    pub max_buffer_size: usize,
}

impl Default for CommonInputConfig {
    fn default() -> Self {
        Self {
            max_buffer_size: 20_000,
        }
    }
}

#[derive(Deserialize, Default)]
pub struct SyslogInputConfig {
    #[serde(flatten)]
    pub common: CommonInputConfig,
    pub exclusion_filters: Vec<SyslogExclusionFilter>,
}

/// Exclusion filter patterns for syslog.
///
/// If more than one pattern is specified, all the pattern specified must match for
/// the log entry to be excluded
#[derive(Deserialize, Default)]
pub struct SyslogExclusionFilter {
    #[serde(with = "serde_regex")]
    pub appname: Option<Regex>,
    #[serde(with = "serde_regex")]
    pub facility: Option<Regex>,
    #[serde(with = "serde_regex")]
    pub message: Option<Regex>,
}

#[derive(Deserialize, Default)]
pub struct GelfInputConfig {
    #[serde(flatten)]
    pub common: CommonInputConfig,
}
