pub mod rlog_service_protocol;

// re-export prost & tonic so all dependents crate will use the right prost/tonic version
pub use prost;
pub use prost_wkt_types;
pub use tonic;
