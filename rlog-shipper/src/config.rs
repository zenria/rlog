use arc_swap::ArcSwap;
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

use self::eqregex::EqRegex;

lazy_static! {
    pub static ref CONFIG: ArcSwap<Config> = ArcSwap::new(Arc::new(Config::default()));
}

#[derive(Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Config {
    pub syslog_in: Option<SyslogInputConfig>,
    pub gelf_in: Option<GelfInputConfig>,
    pub grpc_out: Option<GrpcOutConfig>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub files_in: HashMap<String, FileParseConfig>,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(Deserialize, Serialize, PartialEq, Eq)]
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

#[derive(Deserialize, Default, Serialize, PartialEq, Eq)]
pub struct SyslogInputConfig {
    #[serde(flatten, default)]
    pub common: CommonInputConfig,
    pub exclusion_filters: Vec<SyslogExclusionFilter>,
}

/// Exclusion filter patterns for syslog.
///
/// If more than one pattern is specified, all the pattern specified must match for
/// the log entry to be excluded
#[derive(Deserialize, Default, Serialize, PartialEq, Eq)]
pub struct SyslogExclusionFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appname: Option<EqRegex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facility: Option<EqRegex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<EqRegex>,
}

pub mod eqregex {
    use regex::Regex;
    use serde::{Deserialize, Serialize};
    use std::ops::Deref;

    #[derive(Clone, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct EqRegex {
        #[serde(with = "serde_regex")]
        inner: Regex,
    }

    impl EqRegex {
        pub fn new(regex: &str) -> Result<Self, regex::Error> {
            Ok(Self {
                inner: Regex::new(regex)?,
            })
        }
    }
    impl PartialEq for EqRegex {
        fn eq(&self, other: &Self) -> bool {
            self.inner.as_str() == other.inner.as_str()
        }
    }
    impl Eq for EqRegex {}

    impl Deref for EqRegex {
        type Target = Regex;

        fn deref(&self) -> &Self::Target {
            &self.inner
        }
    }
}

#[derive(Deserialize, Default, Serialize, PartialEq, Eq)]
pub struct GelfInputConfig {
    #[serde(flatten, default)]
    pub common: CommonInputConfig,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct FileParseConfig {
    #[serde(flatten)]
    pub mapping: FileMappingConfig,
    pub static_fields: HashMap<String, Value>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
#[serde(tag = "mode")]
pub enum FileMappingConfig {
    #[serde(rename = "regex")]
    Regex {
        pattern: EqRegex,
        /// each group of the regex will be mapped to those names ;
        mapping: Vec<FieldMapping>,
    },
    // TODO json
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct FieldMapping {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldType,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
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
