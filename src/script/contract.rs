//! The contract a script must honour, and the protocol around it.

use crate::catalog::{CheckOutcome, DisplayText, Versions};

/// `# lazy-install: v1`, on a line of its own.
pub const MARKER_PREFIX: &str = "# lazy-install: v";
pub const CONTRACT_VERSION: u32 = 1;

/// `needs_update` exit codes. Neither is enough on its own: the matching token
/// must also have been written by a helper (see `PRELUDE`).
pub const CODE_UPDATE_AVAILABLE: i32 = 0;
pub const CODE_UP_TO_DATE: i32 = 10;
pub const CODE_MISSING_FUNCTION: i32 = 3;

/// The functions lazy-install may call. An allowlist: the name passed to bash
/// comes from here, never from the config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Function {
    NeedsUpdate,
    Update,
}

impl Function {
    pub fn name(self) -> &'static str {
        match self {
            Function::NeedsUpdate => "needs_update",
            Function::Update => "update",
        }
    }

    pub fn mode(self) -> &'static str {
        match self {
            Function::NeedsUpdate => "check",
            Function::Update => "update",
        }
    }
}

/// The bash program run as `bash -c PRELUDE lazy-install <path> <function> <mode>`.
///
/// Everything after `source` goes through `builtin`, so a script defining a
/// function called `printf` or `declare` cannot reroute the protocol. That is a
/// guard against accidents, not against a hostile script, which runs with the
/// user's rights anyway: the nonce is what makes every surprise fail *closed*.
///
/// In check mode the nonce arrives on fd 4 and is never exported, so neither the
/// script's children nor `/proc/<pid>/environ` see it. The result token goes to
/// fd 3, fields separated by US (0x1f) because Debian epochs put `:` in versions.
pub const PRELUDE: &str = r#"
builtin set +o posix 2>/dev/null
__li_mode=$3
if [[ $__li_mode == check ]]; then
  builtin read -r -u 4 __li_nonce || builtin exit 97
  builtin exec 4<&-
else
  __li_nonce=
fi
builtin readonly __li_nonce __li_mode
li_up_to_date() {
  if [[ -n $__li_nonce ]]; then
    builtin printf 'LI_RESULT\x1f%s\x1fup-to-date\x1f%s\x1f%s\n' "$__li_nonce" "${1-}" "${2-}" >&3
  fi
  builtin return 10
}
li_update_available() {
  if [[ -n $__li_nonce ]]; then
    builtin printf 'LI_RESULT\x1f%s\x1fupdate-available\x1f%s\x1f%s\n' "$__li_nonce" "${1-}" "${2-}" >&3
  fi
  builtin return 0
}
builtin readonly -f li_up_to_date li_update_available
builtin export LAZY_INSTALL=1 LAZY_INSTALL_MODE="$__li_mode"
builtin source -- "$1"
if ! builtin declare -F -- "$2" >/dev/null; then
  if [[ -n $__li_nonce ]]; then
    builtin printf 'LI_INVALID\x1f%s\x1fmissing\n' "$__li_nonce" >&3
  else
    builtin printf 'lazy-install: function %s is not defined in %s\n' "$2" "$1" >&2
  fi
  builtin exit 3
fi
"$2"
"#;

/// The skeleton offered when a script does not exist yet.
pub const TEMPLATE: &str = r#"#!/usr/bin/env bash
# lazy-install: v1
#
# Sourced by lazy-install at run time: define functions only, no top-level side
# effects. Never put a token or a password in this file: read it from the
# environment or from `pass`.

# Says whether an update is available. It MUST end with one of the helpers:
#   li_update_available [installed] [latest]   -> "UPDATE"
#   li_up_to_date       [installed] [latest]   -> "OK"
# Anything else (a crash, a plain `return 10`, a timeout after 30 s) shows as
# "error": a failure never passes for "up to date".
needs_update() {
  echo "TODO: implement needs_update in this script" >&2
  return 2
  # Example, for a GitHub release:
  # local installed latest
  # installed="$(mytool --version | awk '{print $2}')"
  # latest="$(curl -fsSI https://github.com/OWNER/REPO/releases/latest \
  #   | sed -n 's#^location:.*/tag/v\([^[:space:]]*\).*#\1#ip')"
  # [[ -n $latest ]] || return 2
  # if [[ $installed == "$latest" ]]; then
  #   li_up_to_date "$installed" "$latest"
  # else
  #   li_update_available "$installed" "$latest"
  # fi
}

# Installs the update. Runs in a real terminal inside lazy-install: sudo
# prompts and progress bars work. Exit 0 on success.
update() {
  echo "TODO: install the update"
  return 1
}
"#;

/// What fd 3 said, judged against the nonce of this run.
#[derive(Debug, PartialEq, Eq)]
enum Token {
    UpToDate(Versions),
    UpdateAvailable(Versions),
    Missing,
}

/// Exactly one well-formed line carrying the right nonce, or nothing.
///
/// Two lines are refused rather than "the last one wins": a script that calls
/// both helpers has not given an answer.
fn parse_token(fd3: &[u8], nonce: &str) -> Option<Token> {
    let text = String::from_utf8_lossy(fd3);
    let lines: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
    if lines.len() != 1 {
        return None;
    }
    let fields: Vec<&str> = lines[0].split('\x1f').collect();
    match fields.as_slice() {
        ["LI_RESULT", n, state, installed, latest] if *n == nonce => match *state {
            "up-to-date" => Some(Token::UpToDate(Versions::new(installed, latest))),
            "update-available" => Some(Token::UpdateAvailable(Versions::new(installed, latest))),
            _ => None,
        },
        ["LI_INVALID", n, "missing"] if *n == nonce => Some(Token::Missing),
        _ => None,
    }
}

/// The decision table: the exit code AND the token must agree.
pub fn decide(
    code: Option<i32>,
    fd3: &[u8],
    nonce: &str,
    function: Function,
    stderr: &[u8],
) -> CheckOutcome {
    let token = parse_token(fd3, nonce);
    match (code, token) {
        (Some(CODE_UPDATE_AVAILABLE), Some(Token::UpdateAvailable(v))) => {
            CheckOutcome::UpdateAvailable(v)
        }
        (Some(CODE_UP_TO_DATE), Some(Token::UpToDate(v))) => CheckOutcome::UpToDate(v),
        (Some(CODE_MISSING_FUNCTION), Some(Token::Missing)) => CheckOutcome::Invalid(
            DisplayText::message(&format!("function {} is not defined", function.name())),
        ),
        (code, _) => CheckOutcome::Errored(error_message(code, stderr)),
    }
}

/// The last non-empty stderr line, or what is known about the exit.
fn error_message(code: Option<i32>, stderr: &[u8]) -> DisplayText {
    let text = String::from_utf8_lossy(stderr);
    if let Some(line) = text.lines().rev().map(str::trim).find(|l| !l.is_empty()) {
        return DisplayText::message(line);
    }
    match code {
        Some(CODE_UP_TO_DATE) | Some(CODE_UPDATE_AVAILABLE) => DisplayText::message(
            "no result: needs_update must end with li_up_to_date or li_update_available",
        ),
        Some(c) => DisplayText::message(&format!("needs_update exited with {c}")),
        None => DisplayText::message("needs_update was killed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const N: &str = "abc";

    fn tok(state: &str, i: &str, l: &str) -> Vec<u8> {
        format!("LI_RESULT\x1f{N}\x1f{state}\x1f{i}\x1f{l}\n").into_bytes()
    }

    #[test]
    fn code_and_token_must_agree() {
        let f = Function::NeedsUpdate;
        assert!(matches!(
            decide(Some(0), &tok("update-available", "1", "2"), N, f, b""),
            CheckOutcome::UpdateAvailable(_)
        ));
        assert!(matches!(
            decide(Some(10), &tok("up-to-date", "1", "1"), N, f, b""),
            CheckOutcome::UpToDate(_)
        ));
        // a bare `return 10`
        assert!(matches!(
            decide(Some(10), b"", N, f, b""),
            CheckOutcome::Errored(_)
        ));
        // code and token disagree
        assert!(matches!(
            decide(Some(10), &tok("update-available", "1", "2"), N, f, b""),
            CheckOutcome::Errored(_)
        ));
        // forged nonce
        let forged = b"LI_RESULT\x1fzzz\x1fup-to-date\x1f1\x1f1\n";
        assert!(matches!(
            decide(Some(10), forged, N, f, b""),
            CheckOutcome::Errored(_)
        ));
        // two answers
        let mut two = tok("up-to-date", "1", "1");
        two.extend(tok("up-to-date", "1", "1"));
        assert!(matches!(
            decide(Some(10), &two, N, f, b""),
            CheckOutcome::Errored(_)
        ));
    }

    #[test]
    fn code_3_is_invalid_only_with_its_token() {
        let f = Function::NeedsUpdate;
        let missing = format!("LI_INVALID\x1f{N}\x1fmissing\n");
        assert!(matches!(
            decide(Some(3), missing.as_bytes(), N, f, b""),
            CheckOutcome::Invalid(_)
        ));
        assert!(matches!(
            decide(Some(3), b"", N, f, b"curl: (3) URL malformed"),
            CheckOutcome::Errored(_)
        ));
    }

    #[test]
    fn versions_with_colons_survive() {
        match decide(
            Some(10),
            &tok("up-to-date", "1:2.3", "1:2.3"),
            N,
            Function::NeedsUpdate,
            b"",
        ) {
            CheckOutcome::UpToDate(v) => assert_eq!(v.label(), "1:2.3"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn errored_carries_the_last_stderr_line() {
        match decide(Some(1), b"", N, Function::NeedsUpdate, b"first\nboom\n\n") {
            CheckOutcome::Errored(m) => assert_eq!(m.as_str(), "boom"),
            other => panic!("{other:?}"),
        }
    }
}
