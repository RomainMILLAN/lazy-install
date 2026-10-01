//! The plain-text copy of a run, `~/.local/state/lazy-install/logs/<slug>.log`.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

use crate::script::trust::check_dir;

/// 10 MiB per run; a chatty update replayed often must not fill the disk.
const LOG_CAP: u64 = 10 * 1024 * 1024;

pub struct LogSink {
    writer: strip_ansi_escapes::Writer<File>,
    written: u64,
    truncated: bool,
}

impl LogSink {
    /// Opens (truncating) the log of the last run. The directory is created 700
    /// and checked like a script directory; the file is never followed through a
    /// symlink and is 600 even if it existed before.
    pub fn open(path: &Path) -> std::io::Result<LogSink> {
        let dir = path
            .parent()
            .ok_or_else(|| std::io::Error::other("log path has no directory"))?;
        if !dir.exists() {
            fs::create_dir_all(dir)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        }
        check_dir(dir).map_err(|e| std::io::Error::other(e.to_string()))?;
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(nix::libc::O_NOFOLLOW)
            .open(path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(LogSink {
            writer: strip_ansi_escapes::Writer::new(file),
            written: 0,
            truncated: false,
        })
    }

    pub fn write(&mut self, bytes: &[u8]) {
        if self.truncated {
            return;
        }
        let room = LOG_CAP.saturating_sub(self.written) as usize;
        if bytes.len() > room {
            let _ = self.writer.write_all(&bytes[..room]);
            let _ = self.writer.write_all(b"\n[log truncated]\n");
            self.truncated = true;
            return;
        }
        if self.writer.write_all(bytes).is_ok() {
            self.written += bytes.len() as u64;
        }
    }

    pub fn finish(mut self, footer: &str) {
        let _ = self.writer.write_all(format!("\n{footer}\n").as_bytes());
        let _ = self.writer.flush();
    }
}
