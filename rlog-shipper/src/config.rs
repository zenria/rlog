use arc_swap::ArcSwap;
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};

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
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub files_in: HashMap<String, FileParseConfig>,
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

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "mode")]
pub enum FileParseConfig {
    #[serde(rename = "regex")]
    Regex {
        #[serde(with = "serde_regex")]
        pattern: Regex,
        /// each group of the regex will be mapped to those names ;
        mapping: Vec<FieldMapping>,
    },
    // TODO json
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FieldMapping {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Timestamp,
    Number,
    String,
    SyslogLevelText,
}
