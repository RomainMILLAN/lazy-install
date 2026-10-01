//! Loads and saves the catalog. Never writes what it could not read, never
//! overwrites what changed behind its back.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::fcntl::{Flock, FlockArg};
use sha2::{Digest, Sha256};

use crate::catalog::{AppName, Catalog};
use crate::script::trust::{check_file, TrustError};

use super::file::{AppEntry, ConfigFile};
use super::paths::{expand, home, resolve_script, DEFAULT_SCRIPTS_DIR};

const LOCK_WAIT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("{0}")]
    NotTrusted(#[from] TrustError),
    #[error("{path}:{line}:{column}: {message}")]
    Parse {
        path: String,
        line: usize,
        column: usize,
        message: String,
    },
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Io(String),
    #[error("config changed on disk, restart to reload")]
    ChangedOnDisk,
    #[error("config is locked by another lazy-install")]
    Locked,
}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self {
        ConfigError::Io(e.to_string())
    }
}

/// Settings held by the store, written back unchanged: the session has no use
/// for them and the TUI does not edit them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    scripts_dir_raw: Option<String>,
    max_parallel_raw: Option<u8>,
    scripts_dir: PathBuf,
}

impl Settings {
    pub fn scripts_dir(&self) -> &Path {
        &self.scripts_dir
    }

    pub fn max_parallel_checks(&self) -> usize {
        self.max_parallel_raw.unwrap_or(4) as usize
    }
}

pub struct ConfigStore {
    path: PathBuf,
    settings: Settings,
    /// sha256 of the file as last read or written; `None` when it did not exist.
    fingerprint: Option<[u8; 32]>,
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn current_fingerprint(path: &Path) -> Result<Option<[u8; 32]>, ConfigError> {
    match fs::read(path) {
        Ok(b) => Ok(Some(digest(&b))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

impl ConfigStore {
    /// A missing file is an empty config. An unreadable, unknown or untrusted
    /// one is an error, and the file is left untouched: `lazy-scp` fell back to
    /// an empty default here, which the next save turned into an erased config.
    pub fn load(path: &Path) -> Result<(ConfigStore, Catalog), ConfigError> {
        if !path.exists() {
            let settings = Self::settings(None, None)?;
            return Ok((
                ConfigStore {
                    path: path.to_path_buf(),
                    settings,
                    fingerprint: None,
                },
                Catalog::empty(),
            ));
        }
        let canonical = check_file(path)?;
        let bytes = fs::read(&canonical)?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|_| ConfigError::Invalid(format!("{}: not UTF-8", path.display())))?;
        let file: ConfigFile = serde_json::from_str(&text).map_err(|e| ConfigError::Parse {
            path: path.display().to_string(),
            line: e.line(),
            column: e.column(),
            message: e.to_string(),
        })?;
        let settings = Self::settings(file.scripts_dir.clone(), file.max_parallel_checks)?;
        let catalog = Self::to_catalog(&file.apps, settings.scripts_dir())?;
        Ok((
            ConfigStore {
                path: path.to_path_buf(),
                settings,
                fingerprint: Some(digest(&bytes)),
            },
            catalog,
        ))
    }

    fn settings(
        scripts_dir: Option<String>,
        max_parallel: Option<u8>,
    ) -> Result<Settings, ConfigError> {
        if let Some(n) = max_parallel {
            if !(1..=16).contains(&n) {
                return Err(ConfigError::Invalid(format!(
                    "max_parallel_checks must be between 1 and 16, got {n}"
                )));
            }
        }
        let raw = scripts_dir
            .clone()
            .unwrap_or_else(|| DEFAULT_SCRIPTS_DIR.to_string());
        Ok(Settings {
            scripts_dir: expand(&raw, &home()),
            scripts_dir_raw: scripts_dir,
            max_parallel_raw: max_parallel,
        })
    }

    fn to_catalog(apps: &[AppEntry], scripts_dir: &Path) -> Result<Catalog, ConfigError> {
        let mut entries = Vec::new();
        for (i, app) in apps.iter().enumerate() {
            let name = AppName::parse(&app.name)
                .map_err(|e| ConfigError::Invalid(format!("apps[{i}].name: {e}")))?;
            if app.script.trim().is_empty() {
                return Err(ConfigError::Invalid(format!("apps[{i}].script is empty")));
            }
            entries.push((name, resolve_script(&app.script, scripts_dir)));
        }
        Catalog::from_entries(entries).map_err(|e| ConfigError::Invalid(e.to_string()))
    }

    fn to_file(&self, catalog: &Catalog) -> ConfigFile {
        ConfigFile {
            scripts_dir: self.settings.scripts_dir_raw.clone(),
            max_parallel_checks: self.settings.max_parallel_raw,
            apps: catalog
                .iter()
                .map(|a| AppEntry {
                    name: a.name().as_str().to_string(),
                    script: a.script().raw().to_string(),
                })
                .collect(),
        }
    }

    pub fn settings_ref(&self) -> &Settings {
        &self.settings
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The real file to replace: through a symlinked config, the target and not
    /// the link.
    fn target(&self) -> Result<PathBuf, ConfigError> {
        if let Ok(c) = self.path.canonicalize() {
            return Ok(c);
        }
        let parent = self.path.parent().unwrap_or(Path::new("."));
        if !parent.exists() {
            fs::create_dir_all(parent)?;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }
        let name = self
            .path
            .file_name()
            .ok_or_else(|| ConfigError::Invalid("config path has no file name".into()))?;
        Ok(parent.canonicalize()?.join(name))
    }

    /// Atomic and serialised: lock, check the fingerprint, write a temporary
    /// file, fsync, rename, refresh the fingerprint.
    pub fn save(&mut self, catalog: &Catalog) -> Result<(), ConfigError> {
        let target = self.target()?;
        let dir = target.parent().unwrap_or(Path::new("/")).to_path_buf();
        let _lock = lock(&dir.join("config.json.lock"))?;

        if current_fingerprint(&target)? != self.fingerprint {
            return Err(ConfigError::ChangedOnDisk);
        }

        let mut json = serde_json::to_string_pretty(&self.to_file(catalog))
            .map_err(|e| ConfigError::Io(e.to_string()))?;
        json.push('\n');

        let tmp = dir.join(format!(".config.json.{}.tmp", std::process::id()));
        let write = || -> std::io::Result<()> {
            let mut f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)?;
            f.write_all(json.as_bytes())?;
            f.sync_all()?;
            fs::rename(&tmp, &target)?;
            File::open(&dir)?.sync_all()
        };
        if let Err(e) = write() {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        self.fingerprint = Some(digest(json.as_bytes()));
        Ok(())
    }
}

/// Non-blocking `flock`, retried for at most a second: the save runs on the UI
/// thread, and a frozen screen is worse than a clear "locked" message.
fn lock(path: &Path) -> Result<Flock<File>, ConfigError> {
    let deadline = Instant::now() + LOCK_WAIT;
    loop {
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(path)?;
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(l) => return Ok(l),
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return Err(ConfigError::Locked),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::ScriptRef;

    fn dir() -> tempfile::TempDir {
        // Never under /tmp: the trust rule refuses it, rightly.
        let d = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).unwrap();
        fs::set_permissions(d.path(), fs::Permissions::from_mode(0o700)).unwrap();
        d
    }

    fn add(c: &Catalog, n: &str) -> Catalog {
        c.with_added(
            AppName::parse(n).unwrap(),
            ScriptRef::new("x.sh", PathBuf::from("/x.sh")),
        )
        .unwrap()
        .0
    }

    #[test]
    fn missing_file_is_empty_and_two_saves_in_a_row_pass() {
        let d = dir();
        let p = d.path().join("config.json");
        let (mut store, cat) = ConfigStore::load(&p).unwrap();
        assert!(cat.is_empty());
        let c1 = add(&cat, "a");
        store.save(&c1).unwrap();
        let c2 = add(&c1, "b");
        store.save(&c2).unwrap();
        let (_, back) = ConfigStore::load(&p).unwrap();
        assert_eq!(back.len(), 2);
        let mode = fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn a_file_changed_behind_our_back_is_not_overwritten() {
        let d = dir();
        let p = d.path().join("config.json");
        let (mut store, cat) = ConfigStore::load(&p).unwrap();
        store.save(&add(&cat, "a")).unwrap();
        fs::write(&p, "{\"apps\": []}\n").unwrap();
        assert_eq!(store.save(&add(&cat, "b")), Err(ConfigError::ChangedOnDisk));
        assert_eq!(fs::read_to_string(&p).unwrap(), "{\"apps\": []}\n");
    }

    #[test]
    fn broken_json_and_unknown_fields_are_errors() {
        let d = dir();
        let p = d.path().join("config.json");
        fs::write(&p, "{\n  \"apps\": [,]\n}\n").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        match ConfigStore::load(&p) {
            Err(ConfigError::Parse { line, .. }) => assert_eq!(line, 2),
            Err(e) => panic!("{e:?}"),
            Ok(_) => panic!("loaded broken json"),
        }
        fs::write(&p, "{\"apps\": [], \"colour\": 1}").unwrap();
        assert!(matches!(
            ConfigStore::load(&p),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn writable_by_others_is_refused() {
        let d = dir();
        let p = d.path().join("config.json");
        fs::write(&p, "{\"apps\": []}").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o602)).unwrap();
        assert!(matches!(
            ConfigStore::load(&p),
            Err(ConfigError::NotTrusted(_))
        ));
    }

    #[test]
    fn settings_are_bounded_and_written_back_unchanged() {
        let d = dir();
        let p = d.path().join("config.json");
        fs::write(&p, "{\"max_parallel_checks\": 40, \"apps\": []}").unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            ConfigStore::load(&p),
            Err(ConfigError::Invalid(_))
        ));

        fs::write(
            &p,
            "{\"scripts_dir\": \"~/s\", \"max_parallel_checks\": 2, \"apps\": []}",
        )
        .unwrap();
        let (mut store, cat) = ConfigStore::load(&p).unwrap();
        assert_eq!(store.settings_ref().max_parallel_checks(), 2);
        store.save(&add(&cat, "a")).unwrap();
        let text = fs::read_to_string(&p).unwrap();
        assert!(text.contains("\"scripts_dir\": \"~/s\""), "{text}");
        assert!(text.contains("\"max_parallel_checks\": 2"), "{text}");
    }

    #[test]
    fn duplicates_in_the_file_are_errors() {
        let d = dir();
        let p = d.path().join("config.json");
        fs::write(
            &p,
            r#"{"apps": [{"name": "a", "script": "a.sh"}, {"name": "A", "script": "b.sh"}]}"#,
        )
        .unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            ConfigStore::load(&p),
            Err(ConfigError::Invalid(_))
        ));
    }

    #[test]
    fn a_held_lock_fails_after_a_second() {
        let d = dir();
        let p = d.path().join("config.json");
        let (mut store, cat) = ConfigStore::load(&p).unwrap();
        let held = lock(&d.path().canonicalize().unwrap().join("config.json.lock")).unwrap();
        assert_eq!(store.save(&add(&cat, "a")), Err(ConfigError::Locked));
        drop(held);
        store.save(&add(&cat, "a")).unwrap();
    }
}
