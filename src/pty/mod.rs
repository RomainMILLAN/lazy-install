//! The embedded terminal an update runs in.

pub mod keys;
pub mod log;
pub mod run;
pub mod screen;
pub mod spawner;

pub use keys::key_to_bytes;
pub use log::LogSink;
pub use run::PtyRun;
pub use screen::Screen;
pub use spawner::PtySpawner;
