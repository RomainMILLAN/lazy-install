//! An application: a name, and the script that knows how to update it.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// Identity of an application for the lifetime of the process only.
///
/// Never persisted: the config file identifies applications by name, and the
/// log files by slug. An id that survived a restart would be a second identity
/// to keep in sync with those two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AppId(u32);

impl AppId {
    pub(crate) fn new(n: u32) -> Self {
        AppId(n)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("the name is empty")]
    Empty,
    #[error("the name is longer than {max} characters")]
    TooLong { max: usize },
    #[error("the name contains a control character")]
    ControlCharacter,
}

/// A validated application name: trimmed, non-empty, printable, bounded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppName(String);

impl AppName {
    pub const MAX: usize = 64;

    pub fn parse(raw: &str) -> Result<Self, NameError> {
        let name = raw.trim();
        if name.is_empty() {
            return Err(NameError::Empty);
        }
        if name.chars().any(char::is_control) {
            return Err(NameError::ControlCharacter);
        }
        if name.chars().count() > Self::MAX {
            return Err(NameError::TooLong { max: Self::MAX });
        }
        Ok(AppName(name.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Names compare without case: `Kitty` and `kitty` are the same application.
    pub fn same_as(&self, other: &AppName) -> bool {
        self.0.to_lowercase() == other.0.to_lowercase()
    }
}

/// A file-name-safe identifier derived from a name, used for the log file.
///
/// Allowlist `[a-z0-9-]`: nothing else can reach a path, so no `/`, no `..`, no
/// leading dot. Two names may produce the same slug (`Foo Bar`, `foo-bar`);
/// `Catalog` refuses that, otherwise two applications would share a log.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Slug(String);

impl Slug {
    pub fn from_name(name: &AppName) -> Self {
        let mut out = String::new();
        let mut dash = false;
        for c in name.as_str().chars().flat_map(char::to_lowercase) {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                out.push(c);
                dash = false;
            } else if !dash && !out.is_empty() {
                out.push('-');
                dash = true;
            }
        }
        while out.ends_with('-') {
            out.pop();
        }
        if out.is_empty() {
            let digest = Sha256::digest(name.as_str().as_bytes());
            let hex: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
            out = format!("app-{hex}");
        }
        Slug(out)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where the script is: as the user wrote it, and resolved to an absolute path.
///
/// `raw` is what goes back into the config file (relative to `scripts_dir`,
/// `~/…`, or absolute), so a config stays portable across machines. `resolved`
/// is what gets checked and run. Resolution is `config::paths`' job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptRef {
    raw: String,
    resolved: PathBuf,
}

impl ScriptRef {
    pub fn new(raw: &str, resolved: PathBuf) -> Self {
        ScriptRef {
            raw: raw.to_string(),
            resolved,
        }
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn path(&self) -> &Path {
        &self.resolved
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Application {
    id: AppId,
    name: AppName,
    slug: Slug,
    script: ScriptRef,
}

impl Application {
    pub(crate) fn new(id: AppId, name: AppName, script: ScriptRef) -> Self {
        let slug = Slug::from_name(&name);
        Application {
            id,
            name,
            slug,
            script,
        }
    }

    pub fn id(&self) -> AppId {
        self.id
    }

    pub fn name(&self) -> &AppName {
        &self.name
    }

    pub fn slug(&self) -> &Slug {
        &self.slug
    }

    pub fn script(&self) -> &ScriptRef {
        &self.script
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slug(s: &str) -> String {
        Slug::from_name(&AppName::parse(s).unwrap())
            .as_str()
            .to_string()
    }

    #[test]
    fn name_is_trimmed_and_validated() {
        assert_eq!(AppName::parse("  kitty ").unwrap().as_str(), "kitty");
        assert_eq!(AppName::parse("  "), Err(NameError::Empty));
        assert_eq!(AppName::parse("a\tb"), Err(NameError::ControlCharacter));
        assert!(AppName::parse(&"x".repeat(65)).is_err());
    }

    #[test]
    fn slug_is_an_allowlist() {
        assert_eq!(slug("Foo Bar"), "foo-bar");
        assert_eq!(slug("foo-bar"), "foo-bar");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug("VS Code (stable)"), "vs-code-stable");
    }

    #[test]
    fn slug_falls_back_to_a_hash_when_nothing_survives() {
        let s = slug("日本語");
        assert!(s.starts_with("app-") && s.len() == 12, "{s}");
    }
}
