//! Runs every check once, headless, and returns the report.

use std::sync::mpsc::{channel, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::catalog::Catalog;
use crate::jobs::{CheckRunner, CheckScheduler};
use crate::session::{Effect, Fact, Session};

use super::report::CheckReport;

/// The deadline for the whole run: one `per_check` per pass of the pool, plus
/// a margin. Only a safety net against a broken runner — a check that was
/// going to answer always fits.
pub fn budget(apps: usize, workers: usize, per_check: Duration) -> Duration {
    let passes = apps.div_ceil(workers.max(1)) as u32;
    per_check * passes + Duration::from_secs(10)
}

/// Schedules every check, collects the results, projects the report.
///
/// The collection does not count answers: it reads until the channel is
/// closed. The scheduler is dropped as soon as everything is queued, so the
/// workers exit when the queue is empty and the last copies of the sink go with
/// them — a runner that panics or a lost result cannot block it. `budget`
/// covers the last case, a runner that never returns.
pub fn collect(
    catalog: Catalog,
    runner: Arc<dyn CheckRunner>,
    workers: usize,
    budget: Duration,
) -> CheckReport {
    let mut session = Session::new(catalog);
    let (tx, rx) = channel::<Fact>();
    let scheduler = CheckScheduler::new(
        runner,
        workers,
        Arc::new(move |app, generation, outcome| {
            let _ = tx.send(Fact::CheckCompleted {
                app,
                generation,
                outcome,
            });
        }),
    );
    for effect in session.request_check_all() {
        match effect {
            Effect::ScheduleCheck {
                app,
                script,
                generation,
            } => scheduler.schedule(app, script, generation),
            // `request_check_all` only ever schedules checks.
            other => unreachable!("request_check_all produced {other:?}"),
        }
    }
    drop(scheduler);

    let deadline = Instant::now() + budget;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(fact) => {
                let effects = session.apply(fact);
                // A completed check never asks for more; ignore, never panic.
                debug_assert!(effects.is_empty(), "{effects:?}");
            }
            Err(RecvTimeoutError::Disconnected) | Err(RecvTimeoutError::Timeout) => break,
        }
    }

    let catalog = session.catalog();
    CheckReport::project(catalog.iter().filter_map(|app| {
        session
            .runtime(app.id())
            .map(|rt| (app.name().as_str(), rt.check()))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{AppName, CheckOutcome, DisplayText, ScriptRef, Versions};
    use std::path::PathBuf;

    /// Answers from the script name: `up`, `old`, `bad`, `panic`, `mute`,
    /// `slow-N` (sleeps N×20 ms, then up to date).
    struct Double;

    impl CheckRunner for Double {
        fn run(&self, script: &ScriptRef) -> CheckOutcome {
            match script.raw() {
                "old" => CheckOutcome::UpdateAvailable(Versions::new("1", "2")),
                "old-slow" => {
                    std::thread::sleep(Duration::from_millis(200));
                    CheckOutcome::UpdateAvailable(Versions::new("3", "4"))
                }
                "bad" => CheckOutcome::Errored(DisplayText::message("boom")),
                "panic" => panic!("runner bug"),
                "mute" => {
                    std::thread::sleep(Duration::from_secs(30));
                    CheckOutcome::UpToDate(Versions::default())
                }
                s if s.starts_with("slow-") => {
                    let n: u64 = s[5..].parse().unwrap();
                    std::thread::sleep(Duration::from_millis(n * 20));
                    CheckOutcome::UpToDate(Versions::default())
                }
                _ => CheckOutcome::UpToDate(Versions::default()),
            }
        }
    }

    fn catalog(scripts: &[&str]) -> Catalog {
        Catalog::from_entries(scripts.iter().enumerate().map(|(i, s)| {
            (
                AppName::parse(&format!("app{i}")).unwrap(),
                ScriptRef::new(s, PathBuf::from(format!("/x/{s}"))),
            )
        }))
        .unwrap()
    }

    fn run(scripts: &[&str], budget: Duration) -> (CheckReport, Duration) {
        let t = Instant::now();
        let r = collect(catalog(scripts), Arc::new(Double), 2, budget);
        (r, t.elapsed())
    }

    #[test]
    fn collects_every_result() {
        let (r, _) = run(&["up", "old", "bad", "up"], Duration::from_secs(5));
        assert!(r.has_updates());
        assert_eq!(
            r.to_text(),
            "UPDATE     app1  1 → 2\nERROR      app2  boom\n1 update, 1 error, 2 up to date\n"
        );
    }

    #[test]
    fn a_panicking_runner_does_not_block_and_reads_no_answer() {
        let (r, took) = run(&["up", "panic", "up"], Duration::from_secs(5));
        assert!(took < Duration::from_secs(2), "{took:?}");
        assert!(r.to_text().contains("NO-ANSWER  app1"), "{}", r.to_text());
    }

    #[test]
    fn a_mute_runner_is_cut_at_the_deadline() {
        let (r, took) = run(&["up", "mute"], Duration::from_millis(300));
        assert!(took < Duration::from_secs(2), "{took:?}");
        assert!(r.to_text().contains("NO-ANSWER  app1"), "{}", r.to_text());
    }

    #[test]
    fn the_report_follows_the_catalog_not_the_arrival_order() {
        // app0 answers last, app1 first: the report still lists app0 first.
        let (r, _) = run(&["old-slow", "old"], Duration::from_secs(5));
        assert_eq!(
            r.to_text(),
            "UPDATE     app0  3 → 4\nUPDATE     app1  1 → 2\n2 updates, 0 up to date\n"
        );
    }

    #[test]
    fn budget_covers_every_pass() {
        let per = Duration::from_secs(36);
        assert_eq!(budget(15, 4, per), per * 4 + Duration::from_secs(10));
        assert_eq!(budget(0, 4, per), Duration::from_secs(10));
        assert_eq!(budget(3, 0, per), per * 3 + Duration::from_secs(10));
    }
}
