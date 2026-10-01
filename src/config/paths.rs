//! Paths as the user writes them (`~/…`, relative to `scripts_dir`, absolute)
//! and as they are run.

use std::path::{Component, Path, PathBuf};

use crate::catalog::ScriptRef;

pub const DEFAULT_SCRIPTS_DIR: &str = "~/.config/lazy-install/scripts";

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home().join(".config"))
        .join("lazy-install")
        .join("config.json")
}

pub fn logs_dir() -> PathBuf {
    dirs::state_dir()
        .unwrap_or_else(|| home().join(".local/state"))
        .join("lazy-install")
        .join("logs")
}

/// `~` and `~/x` expand to the home directory; an absolute path stays; anything
/// else is relative to `base`.
pub fn expand(raw: &str, base: &Path) -> PathBuf {
    let raw = raw.trim();
    if raw == "~" {
        return home();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home().join(rest);
    }
    let p = Path::new(raw);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

pub fn resolve_script(raw: &str, scripts_dir: &Path) -> ScriptRef {
    ScriptRef::new(raw.trim(), expand(raw, scripts_dir))
}

/// Whether `child` is inside `parent`, compared component by component on
/// canonical paths — `/a/bc` is not inside `/a/b`, whatever a string prefix says.
pub fn is_inside(child: &Path, parent: &Path) -> bool {
    match (child.canonicalize(), parent.canonicalize()) {
        (Ok(c), Ok(p)) => c != p && c.starts_with(&p),
        _ => false,
    }
}

/// How to write a path picked in the file browser: relative to `scripts_dir`
/// when inside it, `~/…` under the home directory, absolute otherwise.
pub fn spell(path: &Path, scripts_dir: &Path) -> String {
    if let Ok(rel) = path.strip_prefix(scripts_dir) {
        if rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return rel.display().to_string();
        }
    }
    if let Ok(rel) = path.strip_prefix(home()) {
        return format!("~/{}", rel.display());
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_the_three_spellings() {
        let base = Path::new("/scripts");
        assert_eq!(expand("kitty.sh", base), PathBuf::from("/scripts/kitty.sh"));
        assert_eq!(expand("/opt/x.sh", base), PathBuf::from("/opt/x.sh"));
        assert_eq!(expand("~/x.sh", base), home().join("x.sh"));
    }

    #[test]
    fn spells_paths_back() {
        let sd = Path::new("/scripts");
        assert_eq!(spell(Path::new("/scripts/a/k.sh"), sd), "a/k.sh");
        assert_eq!(spell(&home().join("bin/x.sh"), sd), "~/bin/x.sh");
        assert_eq!(spell(Path::new("/opt/x.sh"), sd), "/opt/x.sh");
    }

    #[test]
    fn inside_is_by_components() {
        let tmp = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        let b = tmp.path().join("b");
        let bc = tmp.path().join("bc");
        std::fs::create_dir(&b).unwrap();
        std::fs::create_dir(&bc).unwrap();
        std::fs::write(bc.join("x.sh"), "").unwrap();
        std::fs::write(b.join("x.sh"), "").unwrap();
        assert!(!is_inside(&bc.join("x.sh"), &b));
        assert!(is_inside(&b.join("x.sh"), &b));
    }
}
