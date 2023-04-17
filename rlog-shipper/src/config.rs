use arc_swap::ArcSwap;
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

lazy_static! {
    pub static ref CONFIG: ArcSwap<Config> = ArcSwap::new(Arc::new(Config::default()));
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub syslog_in: SyslogInputConfig,
    #[serde(default)]
    pub gelf_in: GelfInputConfig,
    #[serde(default)]
    pub grpc_out: GrpcOutConfig,
}

#[derive(Deserialize, Serialize)]
pub struct GrpcOutConfig {
    #[serde(default = "default_buffer_size")]
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

fn default_buffer_size() -> usize {
    20_000
}

#[derive(Deserialize, Serialize)]
pub struct CommonInputConfig {
    /// This will not be hot reloaded (buffer is allocated at the start of the application)
    #[serde(default = "default_buffer_size")]
    pub max_buffer_size: usize,
}

impl Default for CommonInputConfig {
    fn default() -> Self {
        Self {
            max_buffer_size: 20_000,
        }
    }
}

#[derive(Deserialize, Default, Serialize)]
pub struct SyslogInputConfig {
    #[serde(flatten, default)]
    pub common: CommonInputConfig,
    pub exclusion_filters: Vec<SyslogExclusionFilter>,
}

/// Exclusion filter patterns for syslog.
///
/// If more than one pattern is specified, all the pattern specified must match for
/// the log entry to be excluded
#[derive(Deserialize, Default, Serialize)]
pub struct SyslogExclusionFilter {
    #[serde(with = "serde_regex", default, skip_serializing_if = "Option::is_none")]
    pub appname: Option<Regex>,
    #[serde(with = "serde_regex", default, skip_serializing_if = "Option::is_none")]
    pub facility: Option<Regex>,
    #[serde(with = "serde_regex", default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Regex>,
}

#[derive(Deserialize, Default, Serialize)]
pub struct GelfInputConfig {
    #[serde(flatten, default)]
    pub common: CommonInputConfig,
}
