//! Host addressing for engine-owned configuration pages.
pub use mirix_engine_protocol::{
    Configuration, ConfigurationChoice, ConfigurationControl, ConfigurationEvent,
    ConfigurationGroup, ConfigurationKind, ConfigurationPathTarget, ConfigurationValue,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigurationStatus {
    pub input_method: String,
    pub session: String,
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineConfiguration {
    pub status: ConfigurationStatus,
    pub locale: String,
    pub page: Configuration,
    /// Event feedback, absent on ordinary configuration reads.
    pub message: Option<String>,
}
