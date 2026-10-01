//! Static validation of a script against the contract. Executes nothing of the
//! script: a script is read and parsed (`bash -n`), never sourced, so a
//! top-level side effect cannot run just because someone typed a path.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::catalog::ScriptRef;

use super::command::CommandSpec;
use super::contract::{CONTRACT_VERSION, MARKER_PREFIX};
use super::trust::{check_file, TrustError};

const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;
const SYNTAX_TIMEOUT: Duration = Duration::from_secs(5);

/// Bash builtins and keywords. A script may not define a function with one of
/// these names (nor anything starting with `li_`): the prelude relies on them.
const RESERVED: &[&str] = &[
    "alias",
    "bg",
    "bind",
    "break",
    "builtin",
    "caller",
    "cd",
    "command",
    "compgen",
    "complete",
    "compopt",
    "continue",
    "declare",
    "dirs",
    "disown",
    "echo",
    "enable",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "getopts",
    "hash",
    "help",
    "history",
    "jobs",
    "kill",
    "let",
    "local",
    "logout",
    "mapfile",
    "popd",
    "printf",
    "pushd",
    "pwd",
    "read",
    "readarray",
    "readonly",
    "return",
    "set",
    "shift",
    "shopt",
    "source",
    "suspend",
    "test",
    "times",
    "trap",
    "true",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unset",
    "wait",
    "if",
    "then",
    "else",
    "elif",
    "fi",
    "case",
    "esac",
    "for",
    "select",
    "while",
    "until",
    "do",
    "done",
    "in",
    "function",
    "time",
    "coproc",
    "[",
    "[[",
    ".",
    ":",
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    #[error("{0}")]
    Trust(#[from] TrustError),
    #[error("cannot read the script: {0}")]
    Unreadable(String),
    #[error("missing marker line \"{MARKER_PREFIX}{CONTRACT_VERSION}\"")]
    MissingMarker,
    #[error("unsupported contract version \"{0}\" (this lazy-install speaks v{CONTRACT_VERSION})")]
    UnsupportedContractVersion(String),
    #[error("function {0}() is not defined (it must be written out, not generated)")]
    MissingFunction(&'static str),
    #[error("function {0}() uses a reserved name (bash builtin, keyword, or li_*)")]
    ReservedFunctionName(String),
    #[error("syntax error: {0}")]
    Syntax(String),
    #[error("bash -n did not finish within {}s", SYNTAX_TIMEOUT.as_secs())]
    ValidationTimeout,
}

/// Proof that a script satisfied the contract when it was registered.
///
/// Only [`validate`] builds one, and `Session::add`/`edit` accept nothing else:
/// a refused script cannot be registered, even by a caller that skipped the form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedScript {
    script: ScriptRef,
}

impl ValidatedScript {
    pub fn script(&self) -> &ScriptRef {
        &self.script
    }

    pub fn into_script(self) -> ScriptRef {
        self.script
    }

    /// Session tests run without bash or files.
    #[cfg(test)]
    pub(crate) fn unchecked(script: ScriptRef) -> Self {
        ValidatedScript { script }
    }
}

/// Full validation: trust, contract text, `bash -n`.
pub fn validate(script: &ScriptRef) -> Result<ValidatedScript, ContractError> {
    let canonical = check_file(script.path())?;
    let text = read_bounded(&canonical)?;
    check_text(&text)?;
    syntax_check(&canonical)?;
    Ok(ValidatedScript {
        script: script.clone(),
    })
}

/// The textual part alone, re-run by the runners before every execution: a
/// script edited after registration is caught here rather than trusted forever.
pub fn check_text_of(path: &std::path::Path) -> Result<(), ContractError> {
    check_text(&read_bounded(path)?)
}

fn read_bounded(path: &std::path::Path) -> Result<String, ContractError> {
    let file = std::fs::File::open(path).map_err(|e| ContractError::Unreadable(e.to_string()))?;
    let mut bytes = Vec::new();
    file.take(MAX_SCRIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| ContractError::Unreadable(e.to_string()))?;
    if bytes.len() as u64 > MAX_SCRIPT_BYTES {
        return Err(ContractError::Unreadable("larger than 1 MiB".into()));
    }
    String::from_utf8(bytes).map_err(|_| ContractError::Unreadable("not UTF-8".into()))
}

/// Marker, required functions, reserved names. Pure.
pub fn check_text(text: &str) -> Result<(), ContractError> {
    check_marker(text)?;
    let defined = defined_functions(text);
    for name in &defined {
        if RESERVED.contains(&name.as_str()) || name.starts_with("li_") {
            return Err(ContractError::ReservedFunctionName(name.clone()));
        }
    }
    for required in ["needs_update", "update"] {
        if !defined.iter().any(|d| d == required) {
            return Err(ContractError::MissingFunction(required));
        }
    }
    Ok(())
}

fn check_marker(text: &str) -> Result<(), ContractError> {
    let expected = format!("{MARKER_PREFIX}{CONTRACT_VERSION}");
    for line in text.lines() {
        let line = line.trim_end();
        if line == expected {
            return Ok(());
        }
        if let Some(rest) = line.strip_prefix(MARKER_PREFIX) {
            return Err(ContractError::UnsupportedContractVersion(format!(
                "v{rest}"
            )));
        }
    }
    Err(ContractError::MissingMarker)
}

/// Names defined as `name()`, `name ()` or `function name`, on lines that are
/// not comments. Generated definitions (`eval`, a sourced file) are invisible
/// here by design: the contract asks for functions written out.
fn defined_functions(text: &str) -> Vec<String> {
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || "_:.+-[".contains(c);
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("function ") {
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| is_name_char(*c))
                .collect();
            if !name.is_empty() {
                out.push(name);
            }
            continue;
        }
        let name: String = line.chars().take_while(|c| is_name_char(*c)).collect();
        if name.is_empty() {
            continue;
        }
        let after = line[name.len()..].trim_start();
        if let Some(after) = after.strip_prefix('(') {
            if after.trim_start().starts_with(')') {
                out.push(name);
            }
        }
    }
    out
}

fn syntax_check(path: &std::path::Path) -> Result<(), ContractError> {
    let spec = CommandSpec::syntax_check(path);
    let mut child = Command::new(&spec.program)
        .args(&spec.args)
        .env_clear()
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ContractError::Unreadable(format!("cannot run bash: {e}")))?;
    let deadline = Instant::now() + SYNTAX_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ContractError::ValidationTimeout);
            }
        }
    };
    if status.success() {
        return Ok(());
    }
    let mut err = String::new();
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut err);
    }
    let first = err.lines().next().unwrap_or("bash -n failed").to_string();
    Err(ContractError::Syntax(
        crate::catalog::DisplayText::message(&first).to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = "#!/usr/bin/env bash\n# lazy-install: v1\nneeds_update() {\n  :\n}\nfunction update {\n  :\n}\n";

    #[test]
    fn a_conforming_script_passes() {
        assert_eq!(check_text(OK), Ok(()));
    }

    #[test]
    fn marker_is_required_and_versioned() {
        assert_eq!(
            check_text(&OK.replace("# lazy-install: v1\n", "")),
            Err(ContractError::MissingMarker)
        );
        assert_eq!(
            check_text(&OK.replace("v1", "v2")),
            Err(ContractError::UnsupportedContractVersion("v2".into()))
        );
    }

    #[test]
    fn both_functions_are_required() {
        assert_eq!(
            check_text(&OK.replace("needs_update()", "other()")),
            Err(ContractError::MissingFunction("needs_update"))
        );
        // eval-generated definitions are not seen
        let generated = "# lazy-install: v1\neval 'needs_update() { :; }'\nupdate() { :; }\n";
        assert_eq!(
            check_text(generated),
            Err(ContractError::MissingFunction("needs_update"))
        );
    }

    #[test]
    fn reserved_names_are_refused() {
        for bad in ["builtin", "command", "printf", "li_foo", "declare"] {
            let text = format!("{OK}{bad}() {{ :; }}\n");
            assert_eq!(
                check_text(&text),
                Err(ContractError::ReservedFunctionName(bad.into())),
                "{bad}"
            );
        }
    }

    #[test]
    fn comments_do_not_define_functions() {
        let text = "# lazy-install: v1\n# needs_update() {\nupdate() { :; }\n";
        assert_eq!(
            check_text(text),
            Err(ContractError::MissingFunction("needs_update"))
        );
    }
}
