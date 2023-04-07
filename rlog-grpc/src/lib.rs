pub mod rlog_service_protocol;

use std::fmt::{Debug, Display};

// re-export prost & tonic so all dependents crate will use the right prost/tonic version
pub use prost;
pub use prost_wkt_types;
use rlog_service_protocol::SyslogSeverity;
pub use tonic;

impl From<SyslogSeverity> for OTELSeverity {
    fn from(value: SyslogSeverity) -> Self {
        match value {
            SyslogSeverity::Emergency => Self::FATAL4,
            SyslogSeverity::Alert => Self::FATAL3,
            SyslogSeverity::Critical => Self::FATAL,
            SyslogSeverity::Error => Self::ERROR,
            SyslogSeverity::Warning => Self::WARN,
            SyslogSeverity::Notice => Self::INFO3,
            SyslogSeverity::Info => Self::INFO,
            SyslogSeverity::Debug => Self::DEBUG,
        }
    }
}

impl From<u64> for SyslogSeverity {
    fn from(value: u64) -> Self {
        match value {
            0 => Self::Emergency,
            1 => Self::Alert,
            2 => Self::Critical,
            3 => Self::Error,
            4 => Self::Warning,
            5 => Self::Notice,
            6 => Self::Info,
            7 => Self::Debug,
            // fallback to debug for anything else above 7 ;)
            _ => Self::Debug,
        }
    }
}

// OpenTelemetry severity
#[allow(unused)]
#[derive(Debug)]
pub enum OTELSeverity {
    // UNSPECIFIED is the default SeverityNumber, it MUST NOT be used.
    UNSPECIFIED = 0,
    TRACE = 1,
    TRACE2 = 2,
    TRACE3 = 3,
    TRACE4 = 4,
    DEBUG = 5,
    DEBUG2 = 6,
    DEBUG3 = 7,
    DEBUG4 = 8,
    INFO = 9,
    INFO2 = 10,
    INFO3 = 11,
    INFO4 = 12,
    WARN = 13,
    WARN2 = 14,
    WARN3 = 15,
    WARN4 = 16,
    ERROR = 17,
    ERROR2 = 18,
    ERROR3 = 19,
    ERROR4 = 20,
    FATAL = 21,
    FATAL2 = 22,
    FATAL3 = 23,
    FATAL4 = 24,
}

impl Display for OTELSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(&self, f)
    }
}
