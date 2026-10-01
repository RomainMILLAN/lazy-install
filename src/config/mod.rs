//! The repository: `config.json` in, a `Catalog` out, and back.

pub mod file;
pub mod paths;
pub mod store;

pub use store::{ConfigError, ConfigStore, Settings};
