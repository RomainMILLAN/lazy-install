//! The domain: what an application is, which applications exist, and what is
//! currently known about each of them. No UI, no persistence, no bash.

pub mod application;
#[allow(clippy::module_inception)]
pub mod catalog;
pub mod outcome;
pub mod runtime;

pub use application::{AppId, AppName, Application, NameError, ScriptRef, Slug};
pub use catalog::{Catalog, CatalogError};
pub use outcome::{CheckOutcome, DisplayText, ExitOutcome, NotStartedReason, Tag, Versions};
pub use runtime::{AppRuntime, CheckState, UpdateState};
