//! The update queue: a pure state machine, and the only writer of
//! `UpdateState`. It never runs anything; it says what should happen next.

use std::collections::VecDeque;

use crate::catalog::{AppId, ExitOutcome, NotStartedReason};

/// What the queue is told. Some are requests (`Enqueue`, `Forget`, `Quit`),
/// some are reports (`Started`, `Exited`, `NotStarted`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueInput {
    Enqueue(Vec<AppId>),
    Started(AppId),
    Exited(AppId, ExitOutcome),
    NotStarted(AppId, NotStartedReason),
    Forget(AppId),
    QuitRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Spawn(AppId),
    Recheck(AppId),
    MarkQueued(AppId),
    MarkRunning(AppId),
    MarkFinished(AppId, ExitOutcome),
    MarkNotStarted(AppId, NotStartedReason),
    MarkIdle(AppId),
    TerminateActive,
}

#[derive(Debug, Default)]
pub struct UpdateQueue {
    waiting: VecDeque<AppId>,
    /// Spawned (or being spawned): one at a time, sudo and dpkg do not share.
    active: Option<AppId>,
}

impl UpdateQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active(&self) -> Option<AppId> {
        self.active
    }

    pub fn holds(&self, app: AppId) -> bool {
        self.active == Some(app) || self.waiting.contains(&app)
    }

    /// Every consequence of one input, at once.
    pub fn apply(&mut self, input: QueueInput) -> Vec<Action> {
        let mut out = Vec::new();
        match input {
            QueueInput::Enqueue(apps) => {
                for app in apps {
                    if !self.holds(app) {
                        self.waiting.push_back(app);
                        out.push(Action::MarkQueued(app));
                    }
                }
                self.next(&mut out);
            }
            QueueInput::Started(app) => {
                if self.active == Some(app) {
                    out.push(Action::MarkRunning(app));
                }
            }
            QueueInput::Exited(app, outcome) => {
                if self.active == Some(app) {
                    self.active = None;
                    out.push(Action::MarkFinished(app, outcome));
                    out.push(Action::Recheck(app));
                    self.next(&mut out);
                }
            }
            QueueInput::NotStarted(app, reason) => {
                if self.active == Some(app) {
                    self.active = None;
                    out.push(Action::MarkNotStarted(app, reason));
                    self.next(&mut out);
                }
            }
            QueueInput::Forget(app) => {
                if self.active != Some(app) {
                    self.waiting.retain(|a| *a != app);
                    out.push(Action::MarkIdle(app));
                }
            }
            QueueInput::QuitRequested => {
                for app in self.waiting.drain(..) {
                    out.push(Action::MarkIdle(app));
                }
                if self.active.is_some() {
                    out.push(Action::TerminateActive);
                }
            }
        }
        out
    }

    fn next(&mut self, out: &mut Vec<Action>) {
        if self.active.is_none() {
            if let Some(app) = self.waiting.pop_front() {
                self.active = Some(app);
                out.push(Action::Spawn(app));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{AppName, Catalog, ScriptRef};
    use std::path::PathBuf;

    fn ids(n: usize) -> Vec<AppId> {
        let mut c = Catalog::empty();
        let mut out = Vec::new();
        for i in 0..n {
            let (next, id) = c
                .with_added(
                    AppName::parse(&format!("a{i}")).unwrap(),
                    ScriptRef::new("x", PathBuf::from("/x")),
                )
                .unwrap();
            c = next;
            out.push(id);
        }
        out
    }

    #[test]
    fn exit_rechecks_and_starts_the_next_at_once() {
        let [a, b] = ids(2)[..] else { panic!() };
        let mut q = UpdateQueue::new();
        assert_eq!(
            q.apply(QueueInput::Enqueue(vec![a, b])),
            vec![
                Action::MarkQueued(a),
                Action::MarkQueued(b),
                Action::Spawn(a)
            ]
        );
        assert_eq!(
            q.apply(QueueInput::Started(a)),
            vec![Action::MarkRunning(a)]
        );
        assert_eq!(
            q.apply(QueueInput::Exited(a, ExitOutcome::Code(0))),
            vec![
                Action::MarkFinished(a, ExitOutcome::Code(0)),
                Action::Recheck(a),
                Action::Spawn(b)
            ]
        );
    }

    #[test]
    fn enqueue_is_idempotent() {
        let [a] = ids(1)[..] else { panic!() };
        let mut q = UpdateQueue::new();
        q.apply(QueueInput::Enqueue(vec![a]));
        assert_eq!(q.apply(QueueInput::Enqueue(vec![a])), vec![]);
        q.apply(QueueInput::Started(a));
        assert_eq!(q.apply(QueueInput::Enqueue(vec![a])), vec![]);
    }

    #[test]
    fn a_failure_does_not_stop_the_queue() {
        let [a, b] = ids(2)[..] else { panic!() };
        let mut q = UpdateQueue::new();
        q.apply(QueueInput::Enqueue(vec![a, b]));
        let out = q.apply(QueueInput::Exited(a, ExitOutcome::Code(1)));
        assert!(out.contains(&Action::Spawn(b)));
    }

    #[test]
    fn not_started_is_skipped_and_marked() {
        let [a, b] = ids(2)[..] else { panic!() };
        let mut q = UpdateQueue::new();
        q.apply(QueueInput::Enqueue(vec![a, b]));
        let reason = NotStartedReason::new("invalid");
        assert_eq!(
            q.apply(QueueInput::NotStarted(a, reason.clone())),
            vec![Action::MarkNotStarted(a, reason), Action::Spawn(b)]
        );
    }

    #[test]
    fn empty_queue_does_nothing() {
        let mut q = UpdateQueue::new();
        assert_eq!(q.apply(QueueInput::Enqueue(vec![])), vec![]);
        assert_eq!(q.apply(QueueInput::QuitRequested), vec![]);
    }

    #[test]
    fn quit_empties_the_queue_and_terminates_the_active_run() {
        let [a, b] = ids(2)[..] else { panic!() };
        let mut q = UpdateQueue::new();
        q.apply(QueueInput::Enqueue(vec![a, b]));
        assert_eq!(
            q.apply(QueueInput::QuitRequested),
            vec![Action::MarkIdle(b), Action::TerminateActive]
        );
    }

    #[test]
    fn forget_marks_idle() {
        let [a] = ids(1)[..] else { panic!() };
        let mut q = UpdateQueue::new();
        assert_eq!(q.apply(QueueInput::Forget(a)), vec![Action::MarkIdle(a)]);
    }
}
