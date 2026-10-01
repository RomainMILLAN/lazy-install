//! Runs checks on a small pool, so a full `R` does not hit GitHub twenty times
//! at once.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

use crate::catalog::{AppId, CheckOutcome, ScriptRef};

use super::ports::CheckRunner;

/// Where results go: `(app, generation, outcome)`. The generation is carried,
/// never invented, here; only `AppRuntime` hands them out.
pub type CheckSink = Arc<dyn Fn(AppId, u64, CheckOutcome) + Send + Sync>;

struct Job {
    app: AppId,
    script: ScriptRef,
    generation: u64,
}

pub struct CheckScheduler {
    tx: Sender<Job>,
}

impl CheckScheduler {
    pub fn new(runner: Arc<dyn CheckRunner>, workers: usize, sink: CheckSink) -> Self {
        let (tx, rx) = channel::<Job>();
        let rx: Arc<Mutex<Receiver<Job>>> = Arc::new(Mutex::new(rx));
        for _ in 0..workers.max(1) {
            let rx = Arc::clone(&rx);
            let runner = Arc::clone(&runner);
            let sink = Arc::clone(&sink);
            std::thread::spawn(move || loop {
                let job = {
                    let guard = rx.lock().unwrap_or_else(|p| p.into_inner());
                    guard.recv()
                };
                let Ok(job) = job else { break };
                let outcome = runner.run(&job.script);
                sink(job.app, job.generation, outcome);
            });
        }
        CheckScheduler { tx }
    }

    pub fn schedule(&self, app: AppId, script: ScriptRef, generation: u64) {
        let _ = self.tx.send(Job {
            app,
            script,
            generation,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{AppName, Catalog, Versions};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct Slow(AtomicUsize, AtomicUsize);

    impl CheckRunner for Slow {
        fn run(&self, _: &ScriptRef) -> CheckOutcome {
            let now = self.0.fetch_add(1, Ordering::SeqCst) + 1;
            self.1.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(30));
            self.0.fetch_sub(1, Ordering::SeqCst);
            CheckOutcome::UpToDate(Versions::default())
        }
    }

    #[test]
    fn the_pool_is_bounded_and_generations_are_carried() {
        let runner = Arc::new(Slow(AtomicUsize::new(0), AtomicUsize::new(0)));
        let (tx, rx) = channel();
        let sink: CheckSink = Arc::new(move |a, g, _| {
            let _ = tx.send((a, g));
        });
        let sched = CheckScheduler::new(runner.clone(), 2, sink);
        let (c, id) = Catalog::empty()
            .with_added(
                AppName::parse("a").unwrap(),
                ScriptRef::new("a", PathBuf::from("/a")),
            )
            .unwrap();
        let _ = c;
        for g in 0..6 {
            sched.schedule(id, ScriptRef::new("a", PathBuf::from("/a")), g);
        }
        let mut got: Vec<u64> = (0..6).map(|_| rx.recv().unwrap().1).collect();
        got.sort();
        assert_eq!(got, vec![0, 1, 2, 3, 4, 5]);
        assert!(runner.1.load(Ordering::SeqCst) <= 2);
    }
}
