//! The one description of how bash is invoked, shared by the check runner, the
//! PTY runner and the syntax check. Each translates it into its own process
//! type; none builds a command line of its own.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::contract::{Function, PRELUDE};
use super::trust::TrustedScript;

/// Marks every descriptor from `first` up close-on-exec. For `pre_exec` only.
///
/// lazy-install may itself have inherited descriptors without CLOEXEC — from
/// its terminal, an IDE, a CI runner (the GitHub runner hands fds 142 and 145
/// down). Without this they would reach every script. Marking rather than
/// closing keeps std's own CLOEXEC error pipe working until the exec.
///
/// # Safety
/// Only async-signal-safe calls (`close_range`, `fcntl`): fit for `pre_exec`.
pub(crate) unsafe fn cloexec_from(first: i32) {
    use nix::libc;
    // CLOSE_RANGE_CLOEXEC (Linux 5.11+).
    const CLOSE_RANGE_CLOEXEC: libc::c_uint = 1 << 2;
    let done = libc::syscall(
        libc::SYS_close_range,
        first as libc::c_uint,
        libc::c_uint::MAX,
        CLOSE_RANGE_CLOEXEC,
    ) == 0;
    if !done {
        let max = match libc::sysconf(libc::_SC_OPEN_MAX) {
            n if n > 0 => n.min(65_536) as i32,
            _ => 1024,
        };
        for fd in first..max {
            let flags = libc::fcntl(fd, libc::F_GETFD);
            if flags >= 0 {
                libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC);
            }
        }
    }
}

/// Absolute on purpose: `PATH` is the user's, and `bash` must not depend on it.
pub const BASH: &str = "/bin/bash";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// The complete environment of the child, already cleaned.
    pub env: Vec<(OsString, OsString)>,
}

/// Variables bash would act on before our prelude runs, or that belong to us.
fn is_unsafe_var(key: &OsStr) -> bool {
    let k = key.to_string_lossy();
    matches!(
        k.as_ref(),
        "BASH_ENV" | "ENV" | "SHELLOPTS" | "BASHOPTS" | "PS4" | "CDPATH" | "GLOBIGNORE"
    ) || k.starts_with("BASH_FUNC_")
        || k.starts_with("LI_")
        || k.starts_with("LAZY_INSTALL")
}

/// The parent's environment minus everything bash would import or act on.
pub fn cleaned_env(term: &str) -> Vec<(OsString, OsString)> {
    clean(std::env::vars_os(), term)
}

fn clean(
    vars: impl Iterator<Item = (OsString, OsString)>,
    term: &str,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = vars
        .filter(|(k, _)| !is_unsafe_var(k) && k != "TERM")
        .collect();
    env.push(("TERM".into(), term.into()));
    env.sort();
    env
}

impl CommandSpec {
    /// `bash -c PRELUDE lazy-install <canonical path> <function> <mode>`. The
    /// path is a positional argument: it is never interpolated into code.
    pub fn run(script: &TrustedScript, function: Function, term: &str) -> Self {
        Self::run_path(script.path(), function, term)
    }

    fn run_path(path: &Path, function: Function, term: &str) -> Self {
        CommandSpec {
            program: BASH.into(),
            args: vec![
                "--noprofile".into(),
                "--norc".into(),
                "-c".into(),
                PRELUDE.into(),
                "lazy-install".into(),
                path.into(),
                function.name().into(),
                function.mode().into(),
            ],
            env: cleaned_env(term),
        }
    }

    /// `bash -n <path>`: parses, executes nothing.
    pub fn syntax_check(path: &Path) -> Self {
        CommandSpec {
            program: BASH.into(),
            args: vec![
                "--noprofile".into(),
                "--norc".into(),
                "-n".into(),
                path.into(),
            ],
            env: cleaned_env("dumb"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_is_cleaned() {
        let vars = vec![
            ("BASH_ENV", "/x"),
            ("ENV", "/x"),
            ("SHELLOPTS", "xtrace"),
            ("BASHOPTS", "x"),
            ("PS4", "$(x)"),
            ("CDPATH", "/x"),
            ("GLOBIGNORE", "*"),
            ("BASH_FUNC_ls%%", "() { :; }"),
            ("LI_NONCE", "x"),
            ("LAZY_INSTALL", "1"),
            ("TERM", "xterm"),
            ("HOME", "/home/me"),
            ("PATH", "/usr/bin"),
        ]
        .into_iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let env = clean(vars, "dumb");
        let keys: Vec<String> = env
            .iter()
            .map(|(k, _)| k.to_string_lossy().into())
            .collect();
        assert_eq!(keys, vec!["HOME", "PATH", "TERM"]);
        assert_eq!(env[2].1, "dumb");
    }

    #[test]
    fn bash_is_absolute_and_path_is_positional() {
        let spec =
            CommandSpec::run_path(Path::new("/a/$(touch x).sh"), Function::NeedsUpdate, "dumb");
        assert_eq!(spec.program, PathBuf::from("/bin/bash"));
        assert_eq!(spec.args[5], OsString::from("/a/$(touch x).sh"));
        assert_eq!(spec.args[6], OsString::from("needs_update"));
        assert_eq!(spec.args[7], OsString::from("check"));
    }
}
