//! The anticorruption layer towards bash: the only module that knows the
//! prelude, the exit codes and the result token.

pub mod check;
pub mod command;
pub mod contract;
pub mod trust;
pub mod validate;

pub use check::BashCheckRunner;
pub use validate::{validate, ContractError, ValidatedScript};
