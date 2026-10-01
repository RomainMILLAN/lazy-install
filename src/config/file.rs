//! The on-disk shape of `config.json`. Strict: an unknown field is an error, not
//! something to drop silently on the next save.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scripts_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_parallel_checks: Option<u8>,
    #[serde(default)]
    pub apps: Vec<AppEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppEntry {
    pub name: String,
    pub script: String,
}
