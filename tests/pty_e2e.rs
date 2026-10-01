//! An update in a real pseudo-terminal.

mod common;

use std::sync::mpsc::channel;
use std::sync::Arc;
use std::time::Duration;

use common::{dir, raw_script, script};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use lazy_install::catalog::ExitOutcome;
use lazy_install::jobs::{RunEvents, UpdateSpawner};
use lazy_install::pty::{key_to_bytes, PtySpawner};

fn events(log: Option<std::path::PathBuf>) -> (RunEvents, std::sync::mpsc::Receiver<ExitOutcome>) {
    let (tx, rx) = channel();
    (
        RunEvents {
            on_output: Arc::new(|| {}),
            on_exit: Box::new(move |o| {
                let _ = tx.send(o);
            }),
            log_path: log,
        },
        rx,
    )
}

#[test]
fn keys_reach_the_script_and_output_reaches_the_screen_and_the_log() {
    let d = dir();
    let s = raw_script(
        d.path(),
        "rw.sh",
        "#!/usr/bin/env bash\n# lazy-install: v1\nneeds_update() { :; }\nupdate() {\n  printf '\\e[31mready\\e[0m\\n'\n  read -r x\n  echo \"got $x\"\n}\n",
    );
    let log = d.path().join("logs").join("rw.log");
    let (ev, rx) = events(Some(log.clone()));
    let mut run = PtySpawner::new().spawn(&s, (10, 60), ev).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    for c in ['a', 'b', 'c'] {
        run.write(&key_to_bytes(
            KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE),
            false,
        ));
    }
    run.write(&key_to_bytes(
        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        false,
    ));
    let outcome = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the run exits");
    assert_eq!(outcome, ExitOutcome::Code(0));
    let text = run.screen().with_screen(|sc| sc.contents());
    assert!(text.contains("got abc"), "{text}");
    let logged = std::fs::read_to_string(&log).unwrap();
    assert!(
        logged.contains("ready") && !logged.contains("\x1b["),
        "{logged:?}"
    );
    assert!(logged.contains("[lazy-install] exit 0"), "{logged:?}");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&log).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn a_missing_update_function_fails_with_exit_3_and_says_why() {
    let d = dir();
    let s = raw_script(
        d.path(),
        "nu.sh",
        "# lazy-install: v1\nneeds_update() { :; }\nupdate() { :; }\nunset -f update\n",
    );
    let (ev, rx) = events(None);
    let run = PtySpawner::new().spawn(&s, (10, 80), ev).unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        ExitOutcome::Code(3)
    );
    let text = run.screen().with_screen(|sc| sc.contents());
    assert!(text.contains("function update is not defined"), "{text}");
}

#[test]
fn terminate_hangs_up_then_kills() {
    let d = dir();
    let s = raw_script(
        d.path(),
        "long.sh",
        "# lazy-install: v1\nneeds_update() { :; }\nupdate() { trap '' HUP; sleep 60; }\n",
    );
    let (ev, rx) = events(None);
    let mut run = PtySpawner::new().spawn(&s, (10, 80), ev).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    run.terminate(Duration::from_millis(300));
    let outcome = rx.recv_timeout(Duration::from_secs(5)).expect("killed");
    assert!(matches!(outcome, ExitOutcome::Signal(_)), "{outcome:?}");
}

#[test]
fn an_untrusted_script_never_starts() {
    let d = dir();
    let s = script(d.path(), "bad.sh", "needs_update() { :; }");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(s.path(), std::fs::Permissions::from_mode(0o666)).unwrap();
    let (ev, _rx) = events(None);
    let err = PtySpawner::new()
        .spawn(&s, (10, 80), ev)
        .err()
        .expect("refused");
    assert!(err.0.as_str().contains("chmod o-w"), "{}", err.0);
}
