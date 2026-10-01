//! The single entry point of every intention.

use std::collections::HashMap;

use crate::catalog::{AppId, AppName, AppRuntime, Catalog, CatalogError, UpdateState};
use crate::script::ValidatedScript;

use super::effect::Effect;
use super::fact::Fact;
use super::queue::{Action, QueueInput, UpdateQueue};

/// Why an intention was refused. Facts are never refused; intentions may be.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("a change is still being saved")]
    Busy,
    #[error("no such application")]
    NotFound,
    #[error("an update is queued or running for this application")]
    InFlight,
    #[error("the script is invalid: fix it, or edit the application")]
    InvalidScript,
    #[error("no application has an update available")]
    NothingToUpdate,
    #[error("{0}")]
    Catalog(#[from] CatalogError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Added(AppId),
    Edited(AppId),
    Removed(AppId),
}

/// A candidate catalog waiting for the disk to confirm it, and what it changes.
#[derive(Debug)]
struct Pending {
    candidate: Catalog,
    change: Change,
}

impl Pending {
    fn touches(&self, app: AppId) -> bool {
        matches!(self.change, Change::Added(a) | Change::Edited(a) | Change::Removed(a) if a == app)
    }
}

pub struct Session {
    catalog: Catalog,
    runtimes: HashMap<AppId, AppRuntime>,
    queue: UpdateQueue,
    pending: Option<Pending>,
}

impl Session {
    pub fn new(catalog: Catalog) -> Self {
        let runtimes = catalog
            .ids()
            .into_iter()
            .map(|id| (id, AppRuntime::new()))
            .collect();
        Session {
            catalog,
            runtimes,
            queue: UpdateQueue::new(),
            pending: None,
        }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn runtime(&self, app: AppId) -> Option<&AppRuntime> {
        self.runtimes.get(&app)
    }

    pub fn update_in_progress(&self) -> Option<AppId> {
        self.queue.active()
    }

    // --- intentions -------------------------------------------------------

    /// Save first, apply after: the catalog in memory only changes once the
    /// candidate is on disk, so memory and disk never disagree.
    pub fn add(&mut self, name: AppName, script: ValidatedScript) -> Result<Vec<Effect>, Refusal> {
        self.ensure_not_pending()?;
        let (candidate, id) = self.catalog.with_added(name, script.into_script())?;
        Ok(self.propose(candidate, Change::Added(id)))
    }

    pub fn edit(
        &mut self,
        app: AppId,
        name: AppName,
        script: ValidatedScript,
    ) -> Result<Vec<Effect>, Refusal> {
        self.ensure_not_pending()?;
        let rt = self.runtimes.get(&app).ok_or(Refusal::NotFound)?;
        if !rt.can_edit() {
            return Err(Refusal::InFlight);
        }
        let candidate = self.catalog.with_edited(app, name, script.into_script())?;
        Ok(self.propose(candidate, Change::Edited(app)))
    }

    pub fn remove(&mut self, app: AppId) -> Result<Vec<Effect>, Refusal> {
        self.ensure_not_pending()?;
        let rt = self.runtimes.get(&app).ok_or(Refusal::NotFound)?;
        if !rt.can_remove() {
            return Err(Refusal::InFlight);
        }
        let candidate = self.catalog.with_removed(app)?;
        Ok(self.propose(candidate, Change::Removed(app)))
    }

    pub fn request_check(&mut self, app: AppId) -> Result<Vec<Effect>, Refusal> {
        if !self.runtimes.contains_key(&app) {
            return Err(Refusal::NotFound);
        }
        Ok(self.schedule_check(app).into_iter().collect())
    }

    pub fn request_check_all(&mut self) -> Vec<Effect> {
        self.catalog
            .ids()
            .into_iter()
            .filter_map(|id| self.schedule_check(id))
            .collect()
    }

    pub fn request_update(&mut self, app: AppId) -> Result<Vec<Effect>, Refusal> {
        if self.pending.as_ref().is_some_and(|p| p.touches(app)) {
            return Err(Refusal::Busy);
        }
        let rt = self.runtimes.get(&app).ok_or(Refusal::NotFound)?;
        if !rt.can_edit() {
            return Err(Refusal::InFlight);
        }
        if !rt.can_update() {
            return Err(Refusal::InvalidScript);
        }
        let actions = self.queue.apply(QueueInput::Enqueue(vec![app]));
        Ok(self.translate(actions))
    }

    /// `U`: every app whose *state* says an update is waiting — not its tag, so
    /// an app whose last update failed is retried.
    pub fn request_update_all(&mut self) -> Result<Vec<Effect>, Refusal> {
        let targets = self.update_targets();
        if targets.is_empty() {
            return Err(Refusal::NothingToUpdate);
        }
        let actions = self.queue.apply(QueueInput::Enqueue(targets));
        Ok(self.translate(actions))
    }

    /// The apps `U` would pick, in list order.
    pub fn update_targets(&self) -> Vec<AppId> {
        self.catalog
            .ids()
            .into_iter()
            .filter(|id| !self.pending.as_ref().is_some_and(|p| p.touches(*id)))
            .filter(|id| self.runtimes.get(id).is_some_and(AppRuntime::wants_update))
            .collect()
    }

    pub fn quit(&mut self) -> Vec<Effect> {
        let actions = self.queue.apply(QueueInput::QuitRequested);
        self.translate(actions)
    }

    // --- facts ------------------------------------------------------------

    pub fn apply(&mut self, fact: Fact) -> Vec<Effect> {
        match fact {
            Fact::CheckCompleted {
                app,
                generation,
                outcome,
            } => {
                if let Some(rt) = self.runtimes.get_mut(&app) {
                    rt.complete_check(generation, outcome);
                }
                Vec::new()
            }
            Fact::UpdateStarted { app } => {
                let actions = self.queue.apply(QueueInput::Started(app));
                self.translate(actions)
            }
            Fact::UpdateNotStarted { app, reason } => {
                let actions = self.queue.apply(QueueInput::NotStarted(app, reason));
                self.translate(actions)
            }
            Fact::UpdateExited { app, outcome } => {
                let actions = self.queue.apply(QueueInput::Exited(app, outcome));
                self.translate(actions)
            }
            Fact::PersistSucceeded => self.commit(),
            Fact::PersistFailed { .. } => {
                self.pending = None;
                Vec::new()
            }
        }
    }

    // --- internals --------------------------------------------------------

    fn ensure_not_pending(&self) -> Result<(), Refusal> {
        match self.pending {
            Some(_) => Err(Refusal::Busy),
            None => Ok(()),
        }
    }

    fn propose(&mut self, candidate: Catalog, change: Change) -> Vec<Effect> {
        let effect = Effect::Persist(candidate.clone());
        self.pending = Some(Pending { candidate, change });
        vec![effect]
    }

    fn commit(&mut self) -> Vec<Effect> {
        let Some(Pending { candidate, change }) = self.pending.take() else {
            return Vec::new();
        };
        self.catalog = candidate;
        let mut out = Vec::new();
        match change {
            Change::Added(app) => {
                self.runtimes.insert(app, AppRuntime::new());
                out.extend(self.schedule_check(app));
            }
            Change::Edited(app) => {
                // The failure, if any, was the old script's.
                let actions = self.queue.apply(QueueInput::Forget(app));
                out.extend(self.translate(actions));
                out.extend(self.schedule_check(app));
            }
            Change::Removed(app) => {
                self.runtimes.remove(&app);
                out.push(Effect::DropRunRecord(app));
            }
        }
        out
    }

    fn schedule_check(&mut self, app: AppId) -> Option<Effect> {
        let script = self.catalog.get(app)?.script().clone();
        let generation = self.runtimes.get_mut(&app)?.begin_check();
        Some(Effect::ScheduleCheck {
            app,
            script,
            generation,
        })
    }

    /// The queue's actions become state changes and effects. The queue is the
    /// only source of `UpdateState`.
    fn translate(&mut self, actions: Vec<Action>) -> Vec<Effect> {
        let mut out = Vec::new();
        for action in actions {
            match action {
                Action::Spawn(app) => {
                    if let Some(a) = self.catalog.get(app) {
                        out.push(Effect::Spawn {
                            app,
                            script: a.script().clone(),
                            slug: a.slug().clone(),
                        });
                    }
                }
                Action::Recheck(app) => out.extend(self.schedule_check(app)),
                Action::MarkQueued(app) => self.set_update(app, UpdateState::Queued),
                Action::MarkRunning(app) => self.set_update(app, UpdateState::Running),
                Action::MarkFinished(app, outcome) => {
                    self.set_update(app, UpdateState::Finished(outcome));
                    out.push(Effect::UpdateEnded(app));
                }
                Action::MarkNotStarted(app, reason) => {
                    self.set_update(app, UpdateState::NotStarted(reason.0));
                    out.push(Effect::UpdateEnded(app));
                }
                Action::MarkIdle(app) => self.set_update(app, UpdateState::Idle),
                Action::TerminateActive => out.push(Effect::TerminateActive),
            }
        }
        out
    }

    fn set_update(&mut self, app: AppId, state: UpdateState) {
        if let Some(rt) = self.runtimes.get_mut(&app) {
            rt.set_update(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CheckOutcome, ExitOutcome, NotStartedReason, ScriptRef, Tag, Versions};
    use std::path::PathBuf;

    fn name(s: &str) -> AppName {
        AppName::parse(s).unwrap()
    }

    fn vs(s: &str) -> ValidatedScript {
        ValidatedScript::unchecked(ScriptRef::new(s, PathBuf::from(format!("/s/{s}"))))
    }

    fn persisted(effects: &[Effect]) -> bool {
        matches!(effects, [Effect::Persist(_)])
    }

    /// A session with `n` apps, all checked and reporting an update.
    fn session(n: usize) -> (Session, Vec<AppId>) {
        let mut s = Session::new(Catalog::empty());
        let mut ids = Vec::new();
        for i in 0..n {
            assert!(persisted(
                &s.add(name(&format!("a{i}")), vs("x.sh")).unwrap()
            ));
            s.apply(Fact::PersistSucceeded);
            ids.push(*s.catalog.ids().last().unwrap());
        }
        for id in &ids {
            let g = s.runtimes.get_mut(id).unwrap().begin_check();
            s.apply(Fact::CheckCompleted {
                app: *id,
                generation: g,
                outcome: CheckOutcome::UpdateAvailable(Versions::new("1", "2")),
            });
        }
        (s, ids)
    }

    fn check_of(effects: &[Effect]) -> Option<(AppId, u64)> {
        effects.iter().find_map(|e| match e {
            Effect::ScheduleCheck {
                app, generation, ..
            } => Some((*app, *generation)),
            _ => None,
        })
    }

    #[test]
    fn add_persists_first_then_checks() {
        let mut s = Session::new(Catalog::empty());
        let e = s.add(name("kitty"), vs("k.sh")).unwrap();
        assert!(persisted(&e));
        assert!(
            s.catalog().is_empty(),
            "nothing applied before the disk agrees"
        );
        let after = s.apply(Fact::PersistSucceeded);
        let (app, g) = check_of(&after).expect("a check is scheduled");
        assert_eq!(g, 1);
        assert_eq!(s.catalog().len(), 1);
        assert_eq!(s.runtime(app).unwrap().tag(), Tag::Checking);
    }

    #[test]
    fn a_failed_save_leaves_everything_unchanged() {
        let mut s = Session::new(Catalog::empty());
        s.add(name("kitty"), vs("k.sh")).unwrap();
        assert!(s
            .apply(Fact::PersistFailed {
                reason: "disk full".into()
            })
            .is_empty());
        assert!(s.catalog().is_empty());
        assert!(s.add(name("kitty"), vs("k.sh")).is_ok(), "no longer busy");
    }

    #[test]
    fn duplicates_are_refused_without_persist() {
        let (mut s, _) = session(1);
        assert!(matches!(
            s.add(name("A0"), vs("y.sh")),
            Err(Refusal::Catalog(CatalogError::DuplicateName(_)))
        ));
        assert!(s.pending.is_none());
    }

    #[test]
    fn one_change_at_a_time() {
        let (mut s, ids) = session(2);
        s.remove(ids[0]).unwrap();
        assert_eq!(s.add(name("b"), vs("b.sh")), Err(Refusal::Busy));
        assert_eq!(s.request_update(ids[0]), Err(Refusal::Busy));
    }

    #[test]
    fn pending_edit_is_excluded_from_update_all() {
        let (mut s, ids) = session(2);
        s.edit(ids[0], name("a0"), vs("new.sh")).unwrap();
        let e = s.request_update_all().unwrap();
        assert!(e
            .iter()
            .any(|e| matches!(e, Effect::Spawn { app, .. } if *app == ids[1])));
        assert!(!s.queue.holds(ids[0]));
    }

    #[test]
    fn edit_during_a_check_starts_a_new_generation() {
        let mut s = Session::new(Catalog::empty());
        s.add(name("a"), vs("a.sh")).unwrap();
        let (app, g1) = check_of(&s.apply(Fact::PersistSucceeded)).unwrap();
        s.edit(app, name("a"), vs("b.sh")).unwrap();
        let (_, g2) = check_of(&s.apply(Fact::PersistSucceeded)).unwrap();
        assert_ne!(g1, g2);
        s.apply(Fact::CheckCompleted {
            app,
            generation: g1,
            outcome: CheckOutcome::UpToDate(Versions::default()),
        });
        assert_eq!(
            s.runtime(app).unwrap().tag(),
            Tag::Checking,
            "g1 is ignored"
        );
        assert_eq!(s.catalog().get(app).unwrap().script().raw(), "b.sh");
    }

    #[test]
    fn editing_a_failed_app_forgets_the_failure() {
        let (mut s, ids) = session(1);
        let a = ids[0];
        s.request_update(a).unwrap();
        s.apply(Fact::UpdateStarted { app: a });
        s.apply(Fact::UpdateExited {
            app: a,
            outcome: ExitOutcome::Code(1),
        });
        assert_eq!(
            s.runtime(a).unwrap().update(),
            &UpdateState::Finished(ExitOutcome::Code(1))
        );
        s.edit(a, name("a0"), vs("fixed.sh")).unwrap();
        s.apply(Fact::PersistSucceeded);
        assert_eq!(s.runtime(a).unwrap().update(), &UpdateState::Idle);
    }

    #[test]
    fn remove_drops_everything_and_late_facts_are_ignored() {
        let (mut s, ids) = session(1);
        let a = ids[0];
        s.remove(a).unwrap();
        assert_eq!(
            s.apply(Fact::PersistSucceeded),
            vec![Effect::DropRunRecord(a)]
        );
        assert!(s.runtime(a).is_none());
        let late = s.apply(Fact::CheckCompleted {
            app: a,
            generation: 99,
            outcome: CheckOutcome::UpToDate(Versions::default()),
        });
        assert!(late.is_empty());
        assert!(s
            .apply(Fact::UpdateExited {
                app: a,
                outcome: ExitOutcome::Code(0)
            })
            .is_empty());
    }

    #[test]
    fn in_flight_apps_cannot_be_edited_or_removed() {
        let (mut s, ids) = session(2);
        s.request_update_all().unwrap();
        assert_eq!(s.edit(ids[0], name("x"), vs("x")), Err(Refusal::InFlight));
        assert_eq!(s.remove(ids[1]), Err(Refusal::InFlight));
    }

    #[test]
    fn not_started_moves_on_to_the_next() {
        let (mut s, ids) = session(2);
        s.request_update_all().unwrap();
        let e = s.apply(Fact::UpdateNotStarted {
            app: ids[0],
            reason: NotStartedReason::new("refused"),
        });
        assert!(e.contains(&Effect::UpdateEnded(ids[0])));
        assert!(e
            .iter()
            .any(|e| matches!(e, Effect::Spawn { app, .. } if *app == ids[1])));
        assert_eq!(s.runtime(ids[0]).unwrap().tag(), Tag::Failed);
    }

    #[test]
    fn exit_ends_the_update_and_rechecks() {
        let (mut s, ids) = session(1);
        let a = ids[0];
        s.request_update(a).unwrap();
        s.apply(Fact::UpdateStarted { app: a });
        assert_eq!(s.runtime(a).unwrap().tag(), Tag::Updating);
        let e = s.apply(Fact::UpdateExited {
            app: a,
            outcome: ExitOutcome::Code(0),
        });
        assert_eq!(e[0], Effect::UpdateEnded(a));
        assert!(check_of(&e).is_some());
    }

    #[test]
    fn check_during_a_run_is_overridden_by_the_final_recheck() {
        let (mut s, ids) = session(1);
        let a = ids[0];
        s.request_update(a).unwrap();
        s.apply(Fact::UpdateStarted { app: a });
        let (_, g_manual) = check_of(&s.request_check(a).unwrap()).unwrap();
        let e = s.apply(Fact::UpdateExited {
            app: a,
            outcome: ExitOutcome::Code(0),
        });
        let (_, g_final) = check_of(&e).unwrap();
        assert!(g_final > g_manual);
        s.apply(Fact::CheckCompleted {
            app: a,
            generation: g_manual,
            outcome: CheckOutcome::UpdateAvailable(Versions::new("1", "2")),
        });
        assert_eq!(s.runtime(a).unwrap().tag(), Tag::Checking);
    }

    #[test]
    fn update_all_retries_a_failed_app_and_refuses_when_empty() {
        let (mut s, ids) = session(1);
        let a = ids[0];
        s.request_update(a).unwrap();
        let e = s.apply(Fact::UpdateExited {
            app: a,
            outcome: ExitOutcome::Code(1),
        });
        let (_, g) = check_of(&e).unwrap();
        s.apply(Fact::CheckCompleted {
            app: a,
            generation: g,
            outcome: CheckOutcome::UpdateAvailable(Versions::new("1", "2")),
        });
        assert_eq!(s.runtime(a).unwrap().tag(), Tag::Failed);
        assert_eq!(s.update_targets(), vec![a], "U retries it: state, not tag");
        let mut empty = Session::new(Catalog::empty());
        assert_eq!(empty.request_update_all(), Err(Refusal::NothingToUpdate));
    }
}
