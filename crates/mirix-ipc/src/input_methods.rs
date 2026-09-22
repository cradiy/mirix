use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputMethodCatalog {
    pub session: String,
    pub revision: u64,
    pub selection_locked: bool,
    pub enabled_order: Vec<String>,
    pub shortcuts: std::collections::BTreeMap<String, String>,
    pub locked_shortcuts: Vec<String>,
    pub entries: Vec<InstalledInputMethod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledInputMethod {
    pub name: String,
    pub directory: Option<PathBuf>,
    pub enabled: bool,
    pub loaded: bool,
    pub error: Option<String>,
}
