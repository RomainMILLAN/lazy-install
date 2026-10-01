//! Dependency directions that the compiler does not enforce inside one crate.

use std::fs;
use std::path::Path;

/// Every `.rs` file under `src/<dir>`, sub-directories included (`ui/panels`,
/// `ui/components`… would otherwise escape the rules).
fn sources(dir: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(dir)];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|x| x == "rs") {
                out.push((
                    path.display().to_string(),
                    fs::read_to_string(&path).unwrap(),
                ));
            }
        }
    }
    out
}

/// The session decides; it must not know the UI, the PTY or the jobs. From the
/// bash layer it takes one type: the proof that a script was validated.
#[test]
fn session_depends_on_no_adapter() {
    for (path, text) in sources("session") {
        for forbidden in ["crate::ui", "crate::pty", "crate::jobs", "crate::config"] {
            assert!(!text.contains(forbidden), "{path} uses {forbidden}");
        }
        for line in text.lines().filter(|l| l.contains("crate::script")) {
            assert!(line.contains("ValidatedScript"), "{path}: {line}");
        }
    }
}

/// The domain depends on nothing of the application.
#[test]
fn catalog_depends_on_nothing() {
    for (path, text) in sources("catalog") {
        for forbidden in [
            "crate::ui",
            "crate::pty",
            "crate::jobs",
            "crate::config",
            "crate::script",
            "crate::session",
        ] {
            assert!(!text.contains(forbidden), "{path} uses {forbidden}");
        }
    }
}

/// The domain ignores every exchange format: the `--check --json` contract is
/// built from DTOs in `cli`.
#[test]
fn catalog_knows_no_serialization() {
    for (path, text) in sources("catalog") {
        assert!(!text.contains("serde"), "{path} mentions serde");
    }
}

/// The headless side gets its runner injected: it never names the bash layer,
/// the PTY or the UI. Textual rule: `jobs/ports.rs` imports `pty::Screen`, so
/// `cli` still compiles with `pty` transitively — splitting `ports.rs` is
/// another job.
#[test]
fn cli_depends_on_no_adapter() {
    for (path, text) in sources("cli") {
        for forbidden in ["crate::ui", "crate::pty", "crate::script"] {
            assert!(!text.contains(forbidden), "{path} uses {forbidden}");
        }
    }
}

#[test]
fn ui_does_not_know_the_headless_side() {
    for (path, text) in sources("ui") {
        assert!(!text.contains("crate::cli"), "{path} uses crate::cli");
    }
}
