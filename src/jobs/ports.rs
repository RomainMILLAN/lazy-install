//! The ports the session's effects are carried out through. Defined on
//! `&ScriptRef`, so the rules above them are tested with doubles that never
//! touch the file system; the concrete adapters (`script::BashCheckRunner`,
//! `pty::PtySpawner`) do the trust check inside.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::catalog::{CheckOutcome, ExitOutcome, NotStartedReason, ScriptRef};
use crate::pty::Screen;

pub trait CheckRunner: Send + Sync {
    fn run(&self, script: &ScriptRef) -> CheckOutcome;
}

/// What a run reports back while and after it runs.
pub struct RunEvents {
    /// New output: redraw.
    pub on_output: Arc<dyn Fn() + Send + Sync>,
    /// The process has exited (or was killed).
    pub on_exit: Box<dyn FnOnce(ExitOutcome) + Send>,
    /// Where the plain-text copy of the output goes.
    pub log_path: Option<PathBuf>,
}

/// A running update. The spawner builds it; this drives it.
pub trait ActiveUpdate: Send {
    fn write(&mut self, bytes: &[u8]);
    fn resize(&mut self, rows: u16, cols: u16);
    /// Hang up, then kill after `grace`.
    fn terminate(&mut self, grace: Duration);
    fn screen(&self) -> Screen;
}

pub trait UpdateSpawner {
    fn spawn(
        &self,
        script: &ScriptRef,
        size: (u16, u16),
        events: RunEvents,
    ) -> Result<Box<dyn ActiveUpdate>, NotStartedReason>;
}
