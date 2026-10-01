//! `lazy-install --check` on the real binary, real bash scripts.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

use common::{dir, script};

fn config(dir: &Path, apps: &[(&str, &str)]) -> std::path::PathBuf {
    let entries: Vec<String> = apps
        .iter()
        .map(|(n, s)| format!(r#"{{"name": "{n}", "script": "{s}"}}"#))
        .collect();
    let path = dir.join("config.json");
    fs::write(
        &path,
        format!(
            r#"{{"scripts_dir": "{}", "apps": [{}]}}"#,
            dir.display(),
            entries.join(", ")
        ),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

fn check(config: &Path, json: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_lazy-install"));
    cmd.arg("--check").arg("--config").arg(config);
    if json {
        cmd.arg("--json");
    }
    cmd.output().unwrap()
}

fn setup() -> tempfile::TempDir {
    let d = dir();
    script(
        d.path(),
        "uptodate.sh",
        "needs_update() { li_up_to_date 1.0 1.0; }",
    );
    script(
        d.path(),
        "outdated.sh",
        "needs_update() { li_update_available 1.0 2.0; }",
    );
    script(
        d.path(),
        "broken.sh",
        "needs_update() { echo boom >&2; exit 1; }",
    );
    d
}

#[test]
fn an_update_exits_0_and_errors_are_listed_apart() {
    let d = setup();
    let cfg = config(
        d.path(),
        &[
            ("up", "uptodate.sh"),
            ("out", "outdated.sh"),
            ("broken", "broken.sh"),
        ],
    );
    let out = check(&cfg, true);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v,
        serde_json::json!({
            "updates": [{"name": "out", "installed": "1.0", "latest": "2.0"}],
            "errors": [{"name": "broken", "kind": "error", "message": "boom"}],
            "up_to_date": 1
        })
    );
}

#[test]
fn no_update_exits_1_even_with_errors() {
    let d = setup();
    let cfg = config(d.path(), &[("up", "uptodate.sh"), ("broken", "broken.sh")]);
    let out = check(&cfg, false);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("ERROR"), "{text}");
    assert!(
        text.ends_with("0 updates, 1 error, 1 up to date\n"),
        "{text}"
    );
}

#[test]
fn nothing_to_check_exits_2() {
    let d = dir();
    assert_eq!(check(&config(d.path(), &[]), false).status.code(), Some(2));

    let broken = d.path().join("broken.json");
    fs::write(&broken, "{ not json").unwrap();
    fs::set_permissions(&broken, fs::Permissions::from_mode(0o600)).unwrap();
    let out = check(&broken, false);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        out.stdout.is_empty(),
        "nothing on stdout when nothing was checked"
    );
}

#[test]
fn usage_conflicts_are_refused() {
    let bin = env!("CARGO_BIN_EXE_lazy-install");
    for args in [
        &["--json"][..],
        &["--check", "--light"],
        &["--check", "--template"],
    ] {
        let out = Command::new(bin).args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}
