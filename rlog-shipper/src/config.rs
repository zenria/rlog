use arc_swap::ArcSwap;
use lazy_static::lazy_static;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

lazy_static! {
    pub static ref CONFIG: ArcSwap<Config> = ArcSwap::new(Arc::new(Config::default()));
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    pub syslog_in: Option<SyslogInputConfig>,
    pub gelf_in: Option<GelfInputConfig>,
    pub grpc_out: Option<GrpcOutConfig>,
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
pub struct FileParseConfig {
    #[serde(flatten)]
    pub mapping: FileMappingConfig,
    pub static_fields: HashMap<String, Value>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "mode")]
pub enum FileMappingConfig {
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

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Timestamp,
    Number,
    String,
    SyslogLevelText,
}

trait ExtendableOption<T> {
    fn extend_option(&mut self, other: Option<T>);
}

impl<T> ExtendableOption<T> for Option<T> {
    fn extend_option(&mut self, other: Option<T>) {
        if other.is_some() {
            *self = other;
        }
    }
}

// config can be extends by itself ;)
//
impl Extend<Config> for Config {
    fn extend<T: IntoIterator<Item = Config>>(&mut self, iter: T) {
        for Config {
            syslog_in,
            gelf_in,
            grpc_out,
            files_in,
        } in iter
        {
            self.syslog_in.extend_option(syslog_in);
            self.gelf_in.extend_option(gelf_in);
            self.grpc_out.extend_option(grpc_out);
            self.files_in.extend(files_in);
        }
    }
}
