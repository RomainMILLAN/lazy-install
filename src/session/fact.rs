use crate::catalog::{AppId, CheckOutcome, ExitOutcome, NotStartedReason};

/// Something that happened, reported back to the session. Never refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fact {
    CheckCompleted {
        app: AppId,
        generation: u64,
        outcome: CheckOutcome,
    },
    UpdateStarted {
        app: AppId,
    },
    UpdateNotStarted {
        app: AppId,
        reason: NotStartedReason,
    },
    UpdateExited {
        app: AppId,
        outcome: ExitOutcome,
    },
    PersistSucceeded,
    PersistFailed {
        reason: String,
    },
}
