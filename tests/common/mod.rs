#![allow(dead_code)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use lazy_install::catalog::ScriptRef;

/// Never under /tmp: the trust rule refuses a 1777 ancestor, and a test that
/// had to weaken the rule to pass would be worse than no test.
pub fn dir() -> tempfile::TempDir {
    let d = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
    d
}

pub fn script(dir: &Path, name: &str, body: &str) -> ScriptRef {
    let path = dir.join(name);
    let text = format!(
        "#!/usr/bin/env bash\n# lazy-install: v1\n{body}\nupdate() {{\n  echo updating\n}}\n"
    );
    fs::write(&path, text).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    ScriptRef::new(name, path)
}

pub fn raw_script(dir: &Path, name: &str, text: &str) -> ScriptRef {
    let path: PathBuf = dir.join(name);
    fs::write(&path, text).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    ScriptRef::new(name, path)
}
