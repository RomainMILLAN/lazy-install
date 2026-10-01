//! Dependency directions that the compiler does not enforce inside one crate.

use std::fs;
use std::path::Path;

fn sources(dir: &str) -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(dir);
    fs::read_dir(&root)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
        .map(|e| {
            (
                e.path().display().to_string(),
                fs::read_to_string(e.path()).unwrap(),
            )
        })
        .collect()
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
