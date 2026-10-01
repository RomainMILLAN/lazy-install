//! Holds the one active run and the last run of every application.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::catalog::{AppId, ExitOutcome, NotStartedReason, ScriptRef, Slug};
use crate::pty::Screen;

use super::ports::{ActiveUpdate, RunEvents, UpdateSpawner};

/// What is kept of a run after it ends: its screen, and how it ended.
pub struct RunRecord {
    pub screen: Screen,
    pub outcome: Option<ExitOutcome>,
}

pub type ExitSink = Arc<dyn Fn(AppId, ExitOutcome) + Send + Sync>;

pub struct UpdateRunner {
    spawner: Box<dyn UpdateSpawner>,
    active: Option<(AppId, Box<dyn ActiveUpdate>)>,
    records: HashMap<AppId, RunRecord>,
    logs_dir: Option<PathBuf>,
}

impl UpdateRunner {
    pub fn new(spawner: Box<dyn UpdateSpawner>, logs_dir: Option<PathBuf>) -> Self {
        UpdateRunner {
            spawner,
            active: None,
            records: HashMap::new(),
            logs_dir,
        }
    }

    pub fn start(
        &mut self,
        app: AppId,
        script: &ScriptRef,
        slug: &Slug,
        size: (u16, u16),
        on_output: Arc<dyn Fn() + Send + Sync>,
        on_exit: ExitSink,
    ) -> Result<(), NotStartedReason> {
        if self.active.is_some() {
            return Err(NotStartedReason::new("another update is running"));
        }
        let events = RunEvents {
            on_output,
            on_exit: Box::new(move |outcome| on_exit(app, outcome)),
            log_path: self
                .logs_dir
                .as_ref()
                .map(|d| d.join(format!("{}.log", slug.as_str()))),
        };
        let run = self.spawner.spawn(script, size, events)?;
        self.records.insert(
            app,
            RunRecord {
                screen: run.screen(),
                outcome: None,
            },
        );
        self.active = Some((app, run));
        Ok(())
    }

    pub fn active_app(&self) -> Option<AppId> {
        self.active.as_ref().map(|(a, _)| *a)
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if let Some((_, run)) = self.active.as_mut() {
            run.write(bytes);
        }
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        if let Some((_, run)) = self.active.as_mut() {
            run.resize(rows, cols);
        }
    }

    pub fn terminate(&mut self, grace: Duration) {
        if let Some((_, run)) = self.active.as_mut() {
            run.terminate(grace);
        }
    }

    /// The process has exited: the run becomes a record.
    pub fn exited(&mut self, app: AppId, outcome: ExitOutcome) {
        if self.active_app() == Some(app) {
            self.active = None;
        }
        if let Some(r) = self.records.get_mut(&app) {
            r.outcome = Some(outcome);
        }
    }

    pub fn drop_record(&mut self, app: AppId) {
        self.records.remove(&app);
    }

    pub fn record(&self, app: AppId) -> Option<&RunRecord> {
        self.records.get(&app)
    }
}
