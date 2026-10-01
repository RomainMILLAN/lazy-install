//! The headless side: `lazy-install --check`, every check without the TUI.
//!
//! Depends on `session`, `jobs` and `catalog`, never on `ui`, `pty` or
//! `script` (the runner is injected by `main`).

pub mod check;
pub mod report;

pub use check::{budget, collect};
pub use report::CheckReport;
