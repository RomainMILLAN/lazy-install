//! The application layer: one entry point for every intention, and the rules
//! that turn intentions and facts into effects. Imports nothing from `ui`.

pub mod effect;
pub mod fact;
pub mod queue;
#[allow(clippy::module_inception)]
pub mod session;

pub use effect::Effect;
pub use fact::Fact;
pub use queue::{Action, QueueInput, UpdateQueue};
pub use session::{Refusal, Session};
