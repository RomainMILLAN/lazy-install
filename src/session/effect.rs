use crate::catalog::{AppId, Catalog, ScriptRef, Slug};

/// Something the session wants done. `Tui` executes these without deciding
/// anything, and reports each result back as a `Fact`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ScheduleCheck {
        app: AppId,
        script: ScriptRef,
        generation: u64,
    },
    Spawn {
        app: AppId,
        script: ScriptRef,
        slug: Slug,
    },
    TerminateActive,
    DropRunRecord(AppId),
    /// Write this candidate catalog. Answered by `PersistSucceeded` or
    /// `PersistFailed`, synchronously, before the next key is read.
    Persist(Catalog),
    /// An update is over (or never started). The view decides what to do with
    /// focus; the session does not know there is a focus.
    UpdateEnded(AppId),
}
