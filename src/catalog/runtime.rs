//! What is currently known about one application, and what may be done to it.

use super::outcome::{CheckOutcome, DisplayText, ExitOutcome, Tag, Versions};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckState {
    Unknown,
    Checking { generation: u64 },
    UpToDate(Versions),
    UpdateAvailable(Versions),
    Errored(DisplayText),
    Invalid(DisplayText),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    Idle,
    Queued,
    Running,
    Finished(ExitOutcome),
    NotStarted(DisplayText),
}

/// Two independent axes: what the last check said, and where the update is.
///
/// They used to be one enum, and queueing an update erased the versions the
/// check had found. Kept apart, a queued app still reads `1.2 → 1.3`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppRuntime {
    check: CheckState,
    update: UpdateState,
    /// Only `AppRuntime` hands out generations; the jobs merely carry them back.
    last_generation: u64,
}

impl Default for AppRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl AppRuntime {
    pub fn new() -> Self {
        AppRuntime {
            check: CheckState::Unknown,
            update: UpdateState::Idle,
            last_generation: 0,
        }
    }

    pub fn check(&self) -> &CheckState {
        &self.check
    }

    pub fn update(&self) -> &UpdateState {
        &self.update
    }

    /// Starts a new check and returns its generation. Any result still in flight
    /// for an older generation will be discarded on arrival.
    pub fn begin_check(&mut self) -> u64 {
        self.last_generation += 1;
        let generation = self.last_generation;
        // The versions of a previous result are kept on screen while checking
        // would be nice, but "checking…" is the honest tag: the old answer may be
        // about a script that has just been edited.
        self.check = CheckState::Checking { generation };
        generation
    }

    /// Applies a check result. Returns false when it was stale and ignored.
    pub fn complete_check(&mut self, generation: u64, outcome: CheckOutcome) -> bool {
        if generation != self.last_generation {
            return false;
        }
        self.check = match outcome {
            CheckOutcome::UpdateAvailable(v) => CheckState::UpdateAvailable(v),
            CheckOutcome::UpToDate(v) => CheckState::UpToDate(v),
            CheckOutcome::Invalid(m) => CheckState::Invalid(m),
            CheckOutcome::Errored(m) => CheckState::Errored(m),
        };
        true
    }

    /// Only `UpdateQueue`'s actions reach this, through the session.
    pub(crate) fn set_update(&mut self, state: UpdateState) {
        self.update = state;
    }

    fn in_flight(&self) -> bool {
        matches!(self.update, UpdateState::Queued | UpdateState::Running)
    }

    pub fn can_edit(&self) -> bool {
        !self.in_flight()
    }

    pub fn can_remove(&self) -> bool {
        !self.in_flight()
    }

    pub fn can_update(&self) -> bool {
        !self.in_flight() && !matches!(self.check, CheckState::Invalid(_))
    }

    /// Whether `U` picks this app. Decided on the state, never on the tag: an app
    /// whose last update failed reads `failed`, yet still has an update waiting.
    pub fn wants_update(&self) -> bool {
        matches!(self.check, CheckState::UpdateAvailable(_)) && self.can_update()
    }

    /// The versions to show, whatever the tag.
    pub fn versions(&self) -> Option<&Versions> {
        match &self.check {
            CheckState::UpToDate(v) | CheckState::UpdateAvailable(v) => Some(v),
            _ => None,
        }
    }

    /// The reason to show next to an `error`, `invalid` or `failed` tag.
    pub fn detail(&self) -> Option<String> {
        match (&self.update, &self.check) {
            (UpdateState::NotStarted(m), _) => Some(m.to_string()),
            (UpdateState::Finished(o), _) if !o.is_success() => Some(o.label()),
            (_, CheckState::Invalid(m) | CheckState::Errored(m)) => Some(m.to_string()),
            _ => None,
        }
    }

    /// Priority projection: the first row that applies wins.
    ///
    /// | 1 | Running                         | updating… |
    /// | 2 | Queued                          | queued    |
    /// | 3 | Invalid                         | invalid   |
    /// | 4 | NotStarted, or Finished failure | failed    |
    /// | 5 | Checking                        | checking… |
    /// | 6 | Errored                         | error     |
    /// | 7 | UpdateAvailable                 | UPDATE    |
    /// | 8 | UpToDate                        | OK        |
    /// | 9 | Unknown                         | —         |
    pub fn tag(&self) -> Tag {
        match (&self.update, &self.check) {
            (UpdateState::Running, _) => Tag::Updating,
            (UpdateState::Queued, _) => Tag::Queued,
            (_, CheckState::Invalid(_)) => Tag::Invalid,
            (UpdateState::NotStarted(_), _) => Tag::Failed,
            (UpdateState::Finished(o), _) if !o.is_success() => Tag::Failed,
            (_, CheckState::Checking { .. }) => Tag::Checking,
            (_, CheckState::Errored(_)) => Tag::Error,
            (_, CheckState::UpdateAvailable(_)) => Tag::Update,
            (_, CheckState::UpToDate(_)) => Tag::Ok,
            (_, CheckState::Unknown) => Tag::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checks() -> Vec<CheckState> {
        vec![
            CheckState::Unknown,
            CheckState::Checking { generation: 1 },
            CheckState::UpToDate(Versions::new("1", "1")),
            CheckState::UpdateAvailable(Versions::new("1", "2")),
            CheckState::Errored(DisplayText::message("boom")),
            CheckState::Invalid(DisplayText::message("bad")),
        ]
    }

    fn updates() -> Vec<UpdateState> {
        vec![
            UpdateState::Idle,
            UpdateState::Queued,
            UpdateState::Running,
            UpdateState::Finished(ExitOutcome::Code(0)),
            UpdateState::Finished(ExitOutcome::Code(1)),
            UpdateState::Finished(ExitOutcome::Signal(9)),
            UpdateState::NotStarted(DisplayText::message("refused")),
        ]
    }

    fn expected(check: &CheckState, update: &UpdateState) -> Tag {
        let failed = matches!(update, UpdateState::NotStarted(_))
            || matches!(update, UpdateState::Finished(o) if !o.is_success());
        if *update == UpdateState::Running {
            Tag::Updating
        } else if *update == UpdateState::Queued {
            Tag::Queued
        } else if matches!(check, CheckState::Invalid(_)) {
            Tag::Invalid
        } else if failed {
            Tag::Failed
        } else {
            match check {
                CheckState::Checking { .. } => Tag::Checking,
                CheckState::Errored(_) => Tag::Error,
                CheckState::UpdateAvailable(_) => Tag::Update,
                CheckState::UpToDate(_) => Tag::Ok,
                _ => Tag::Unknown,
            }
        }
    }

    /// Every cell of `CheckState × UpdateState`.
    #[test]
    fn tag_table_cell_by_cell() {
        for check in checks() {
            for update in updates() {
                let rt = AppRuntime {
                    check: check.clone(),
                    update: update.clone(),
                    last_generation: 1,
                };
                assert_eq!(
                    rt.tag(),
                    expected(&check, &update),
                    "{check:?} × {update:?}"
                );
            }
        }
    }

    #[test]
    fn failed_wins_over_update_but_versions_stay_visible() {
        let rt = AppRuntime {
            check: CheckState::UpdateAvailable(Versions::new("1", "2")),
            update: UpdateState::Finished(ExitOutcome::Code(1)),
            last_generation: 1,
        };
        assert_eq!(rt.tag(), Tag::Failed);
        assert_eq!(rt.versions().unwrap().label(), "1 → 2");
        assert!(rt.wants_update(), "U still picks it");
    }

    #[test]
    fn queued_keeps_versions() {
        let rt = AppRuntime {
            check: CheckState::UpdateAvailable(Versions::new("1", "2")),
            update: UpdateState::Queued,
            last_generation: 1,
        };
        assert_eq!(rt.tag(), Tag::Queued);
        assert!(rt.versions().is_some());
        assert!(!rt.can_edit() && !rt.can_remove() && !rt.can_update());
    }

    #[test]
    fn stale_generation_is_ignored() {
        let mut rt = AppRuntime::new();
        let g1 = rt.begin_check();
        let g2 = rt.begin_check();
        assert!(!rt.complete_check(g1, CheckOutcome::UpToDate(Versions::default())));
        assert_eq!(rt.tag(), Tag::Checking);
        assert!(rt.complete_check(g2, CheckOutcome::UpToDate(Versions::default())));
        assert_eq!(rt.tag(), Tag::Ok);
    }

    #[test]
    fn invalid_cannot_be_updated() {
        let mut rt = AppRuntime::new();
        let g = rt.begin_check();
        rt.complete_check(g, CheckOutcome::Invalid(DisplayText::message("x")));
        assert!(!rt.can_update());
        assert!(rt.can_edit());
    }
}
