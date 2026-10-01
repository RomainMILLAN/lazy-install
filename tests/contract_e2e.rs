//! The contract, on real bash scripts.

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use common::{dir, raw_script, script};
use lazy_install::catalog::{CheckOutcome, ScriptRef};
use lazy_install::jobs::CheckRunner;
use lazy_install::script::{validate, BashCheckRunner, ContractError};

fn run(s: &ScriptRef) -> CheckOutcome {
    BashCheckRunner::new(Duration::from_secs(5)).run(s)
}

fn is_error(o: &CheckOutcome) -> bool {
    matches!(o, CheckOutcome::Errored(_))
}

#[test]
fn helpers_produce_the_two_answers() {
    let d = dir();
    match run(&script(
        d.path(),
        "a.sh",
        "needs_update() { li_update_available 1 2; }",
    )) {
        CheckOutcome::UpdateAvailable(v) => assert_eq!(v.label(), "1 → 2"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        run(&script(
            d.path(),
            "b.sh",
            "needs_update() { li_up_to_date; }"
        )),
        CheckOutcome::UpToDate(_)
    ));
    match run(&script(
        d.path(),
        "c.sh",
        "needs_update() { li_up_to_date 1:2.3 1:2.3; }",
    )) {
        CheckOutcome::UpToDate(v) => assert_eq!(v.label(), "1:2.3"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn nothing_but_a_helper_can_say_up_to_date() {
    let d = dir();
    for (name, body) in [
        ("ret10.sh", "needs_update() { return 10; }"),
        ("exit1.sh", "needs_update() { exit 1; }"),
        ("ret3.sh", "needs_update() { return 3; }"),
        (
            "forged.sh",
            r#"needs_update() { printf 'LI_RESULT\x1fbad\x1fup-to-date\x1f1\x1f1\n' >&3; return 10; }"#,
        ),
        (
            "printf.sh",
            "printf() { :; }\nneeds_update() { li_up_to_date 1 1; }",
        ),
        (
            "declare.sh",
            "declare() { :; }\nneeds_update() { li_up_to_date 1 1; }",
        ),
    ] {
        let s = script(d.path(), name, body);
        // printf/declare are refused by the static check before running.
        let out = run(&s);
        assert!(
            is_error(&out) || matches!(out, CheckOutcome::Invalid(_)),
            "{name}: {out:?}"
        );
        assert!(!matches!(out, CheckOutcome::UpToDate(_)), "{name}");
    }
}

/// `readonly -f`: the redefinition fails, so the answer can only come from the
/// real helper — proven by the versions it carries.
#[test]
fn helpers_cannot_be_redefined() {
    let d = dir();
    let s = script(
        d.path(),
        "redefine.sh",
        "needs_update() { li_up_to_date() { return 10; }; li_up_to_date 9 9; }",
    );
    match run(&s) {
        CheckOutcome::UpToDate(v) => assert_eq!(v.label(), "9"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_nonce_is_not_in_the_environment_of_children() {
    let d = dir();
    let marker = d.path().join("env.txt");
    let body = format!(
        "needs_update() {{ (env; cat /proc/$$/environ | tr '\\0' '\\n') > {} 2>/dev/null; li_up_to_date; }}",
        marker.display()
    );
    assert!(matches!(
        run(&script(d.path(), "n.sh", &body)),
        CheckOutcome::UpToDate(_)
    ));
    let env = fs::read_to_string(&marker).unwrap();
    assert!(!env.contains("LI_NONCE"), "{env}");
    assert!(env.contains("LAZY_INSTALL_MODE=check"));
}

#[test]
fn a_missing_function_is_invalid_at_run_time() {
    let d = dir();
    // Defined textually (passes the static check) but unset before the call.
    let s = script(
        d.path(),
        "m.sh",
        "needs_update() { :; }\nunset -f needs_update",
    );
    match run(&s) {
        CheckOutcome::Invalid(m) => assert!(m.as_str().contains("needs_update"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_hanging_check_times_out_and_leaks_nothing() {
    let d = dir();
    let s = script(
        d.path(),
        "hang.sh",
        "needs_update() { (setsid sleep 999 &); sleep 999; }",
    );
    let runner = BashCheckRunner::new(Duration::from_secs(1));
    let fds = || fs::read_dir("/proc/self/fd").unwrap().count();
    let before = fds();
    for _ in 0..5 {
        let t = Instant::now();
        let out = runner.run(&s);
        assert!(t.elapsed() < Duration::from_secs(3), "{:?}", t.elapsed());
        match out {
            CheckOutcome::Errored(m) => assert!(m.as_str().contains("timeout"), "{m}"),
            other => panic!("{other:?}"),
        }
    }
    assert!(fds() <= before + 1, "fds grew from {before} to {}", fds());
}

#[test]
fn a_background_child_holding_stdout_does_not_turn_an_answer_into_a_timeout() {
    let d = dir();
    let s = script(
        d.path(),
        "bg.sh",
        "needs_update() { (sleep 999 &); li_up_to_date 1 1; }",
    );
    let t = Instant::now();
    assert!(matches!(
        BashCheckRunner::new(Duration::from_secs(10)).run(&s),
        CheckOutcome::UpToDate(_)
    ));
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
}

#[test]
fn a_flood_of_output_does_not_block() {
    let d = dir();
    let s = script(
        d.path(),
        "flood.sh",
        "needs_update() { head -c 10000000 /dev/zero | tr '\\0' x; head -c 1000000 /dev/zero >&2; li_up_to_date; }",
    );
    assert!(matches!(run(&s), CheckOutcome::UpToDate(_)));
}

#[test]
fn only_our_fds_reach_the_check() {
    let d = dir();
    let out = d.path().join("fds.txt");
    let body = format!(
        "needs_update() {{ ls /proc/self/fd > {}; li_up_to_date; }}",
        out.display()
    );
    let slow = script(
        d.path(),
        "slow.sh",
        "needs_update() { sleep 1; li_up_to_date; }",
    );
    let probe = script(d.path(), "probe.sh", &body);
    let t = std::thread::spawn(move || run(&slow));
    std::thread::sleep(Duration::from_millis(200));
    assert!(matches!(run(&probe), CheckOutcome::UpToDate(_)));
    t.join().unwrap();
    let mut fds: Vec<u32> = fs::read_to_string(&out)
        .unwrap()
        .split_whitespace()
        .filter_map(|s| s.parse().ok())
        .collect();
    fds.sort();
    // 0-4, plus the one `ls` opened to read the directory.
    assert!(fds.iter().all(|f| *f <= 5), "{fds:?}");
}

#[test]
fn trust_is_checked_before_running() {
    let d = dir();
    let s = script(d.path(), "o.sh", "needs_update() { li_up_to_date; }");
    fs::set_permissions(s.path(), fs::Permissions::from_mode(0o702)).unwrap();
    assert!(matches!(run(&s), CheckOutcome::Invalid(_)));

    let sub = d.path().join("sub");
    fs::create_dir(&sub).unwrap();
    let s = script(&sub, "p.sh", "needs_update() { li_up_to_date; }");
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o777)).unwrap();
    match run(&s) {
        CheckOutcome::Invalid(m) => assert!(m.as_str().contains("/sub "), "{m}"),
        other => panic!("{other:?}"),
    }
    fs::set_permissions(&sub, fs::Permissions::from_mode(0o700)).unwrap();
}

#[test]
fn scripts_under_tmp_are_refused_and_tmp_is_named() {
    let Ok(tmp) = tempfile::tempdir_in("/tmp") else {
        return;
    };
    fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let s = script(tmp.path(), "t.sh", "needs_update() { li_up_to_date; }");
    match run(&s) {
        CheckOutcome::Invalid(m) => assert!(m.as_str().contains("/tmp "), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_775_script_is_accepted_when_our_group_is_private() {
    use lazy_install::script::trust::{is_private_group, GroupDb, Identity};
    let (Some(me), Some(db)) = (Identity::current(), GroupDb::load()) else {
        return;
    };
    if !is_private_group(me.gid, &me, &db) {
        eprintln!("skipped: the primary group of this user is not private");
        return;
    }
    let d = dir();
    fs::set_permissions(d.path(), fs::Permissions::from_mode(0o775)).unwrap();
    let s = script(d.path(), "g.sh", "needs_update() { li_up_to_date; }");
    fs::set_permissions(s.path(), fs::Permissions::from_mode(0o775)).unwrap();
    assert!(matches!(run(&s), CheckOutcome::UpToDate(_)));
}

#[test]
#[ignore = "needs root to chown"]
fn a_script_owned_by_someone_else_is_refused() {}

#[test]
fn a_hostile_path_is_never_interpreted() {
    let d = dir();
    let pwned = d.path().join("pwned");
    let name = format!("$(touch {}).sh", pwned.display());
    let s = script(
        d.path(),
        &name.replace('/', "_"),
        "needs_update() { li_up_to_date; }",
    );
    let _ = run(&s);
    assert!(!pwned.exists());
}

#[test]
fn bash_env_is_ignored() {
    let d = dir();
    let flag = d.path().join("bash_env_ran");
    let env_file = d.path().join("env.sh");
    fs::write(&env_file, format!("touch {}\n", flag.display())).unwrap();
    std::env::set_var("BASH_ENV", &env_file);
    let s = script(d.path(), "e.sh", "needs_update() { li_up_to_date; }");
    let out = run(&s);
    let _ = validate(&s);
    std::env::remove_var("BASH_ENV");
    assert!(matches!(out, CheckOutcome::UpToDate(_)));
    assert!(!flag.exists());
}

#[test]
fn validation_executes_nothing() {
    let d = dir();
    let flag = d.path().join("ran");
    let s = raw_script(
        d.path(),
        "side.sh",
        &format!(
            "#!/usr/bin/env bash\n# lazy-install: v1\ntouch {}\nneeds_update() {{ :; }}\nupdate() {{ :; }}\n",
            flag.display()
        ),
    );
    assert!(validate(&s).is_ok());
    assert!(!flag.exists());
}

#[test]
fn validation_reports_what_is_wrong() {
    let d = dir();
    let syntax = raw_script(
        d.path(),
        "syn.sh",
        "# lazy-install: v1\nneeds_update() { if; }\nupdate() { :; }\n",
    );
    assert!(matches!(validate(&syntax), Err(ContractError::Syntax(_))));
    let nomarker = raw_script(
        d.path(),
        "nm.sh",
        "needs_update() { :; }\nupdate() { :; }\n",
    );
    assert_eq!(validate(&nomarker), Err(ContractError::MissingMarker));
}

#[test]
fn removing_the_marker_after_registration_makes_it_invalid() {
    let d = dir();
    let s = script(d.path(), "late.sh", "needs_update() { li_up_to_date; }");
    assert!(validate(&s).is_ok());
    let text = fs::read_to_string(s.path())
        .unwrap()
        .replace("# lazy-install: v1\n", "");
    fs::write(s.path(), text).unwrap();
    assert!(matches!(run(&s), CheckOutcome::Invalid(_)));
}
