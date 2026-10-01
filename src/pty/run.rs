//! A running update.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use nix::sys::signal::{killpg, Signal};
use nix::sys::wait::{waitpid, WaitStatus};
use nix::unistd::Pid;
use portable_pty::{Child, MasterPty, PtySize};

use crate::catalog::ExitOutcome;
use crate::jobs::ports::ActiveUpdate;

use super::log::LogSink;
use super::screen::Screen;

pub struct PtyRun {
    pgid: Pid,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    screen: Screen,
    finished: Arc<Mutex<bool>>,
}

impl PtyRun {
    pub(crate) fn start(
        child: Box<dyn Child + Send + Sync>,
        pid: u32,
        master: Box<dyn MasterPty + Send>,
        writer: Box<dyn Write + Send>,
        screen: Screen,
        reader: JoinHandle<Option<LogSink>>,
        on_exit: Box<dyn FnOnce(ExitOutcome) + Send>,
    ) -> PtyRun {
        let pgid = Pid::from_raw(pid as i32);
        let finished = Arc::new(Mutex::new(false));
        let done = Arc::clone(&finished);
        std::thread::spawn(move || {
            // Our own waitpid, for the signal number portable-pty turns into a
            // name. `child` stays alive (unwaited) until then.
            let outcome = match waitpid(pgid, None) {
                Ok(WaitStatus::Exited(_, code)) => ExitOutcome::Code(code),
                Ok(WaitStatus::Signaled(_, sig, _)) => ExitOutcome::Signal(sig as i32),
                _ => ExitOutcome::Code(-1),
            };
            drop(child);
            // Everything still in the pipe is on screen before we report.
            if let Ok(Some(log)) = reader.join() {
                log.finish(&format!("[lazy-install] {}", outcome.label()));
            }
            *done.lock().unwrap_or_else(|p| p.into_inner()) = true;
            on_exit(outcome);
        });
        PtyRun {
            pgid,
            master: Some(master),
            writer: Some(writer),
            screen,
            finished,
        }
    }

    fn is_finished(&self) -> bool {
        *self.finished.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl ActiveUpdate for PtyRun {
    fn write(&mut self, bytes: &[u8]) {
        if let Some(w) = self.writer.as_mut() {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }
    }

    fn resize(&mut self, rows: u16, cols: u16) {
        if let Some(m) = self.master.as_ref() {
            let _ = m.resize(PtySize {
                rows: rows.max(1),
                cols: cols.max(1),
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        self.screen.set_size(rows, cols);
    }

    /// Hang up the session, then kill the group after `grace`.
    ///
    /// The child is a session leader (portable-pty calls setsid), so its pid is
    /// the group to signal. `sudo` with `use_pty` moves the command it runs to a
    /// pty of its own and relays the hangup; a command already running as root
    /// may still finish its work, and that is documented.
    fn terminate(&mut self, grace: Duration) {
        self.writer = None;
        self.master = None;
        let _ = killpg(self.pgid, Signal::SIGHUP);
        let pgid = self.pgid;
        let finished = Arc::clone(&self.finished);
        std::thread::spawn(move || {
            let step = Duration::from_millis(50);
            let mut waited = Duration::ZERO;
            while waited < grace {
                if *finished.lock().unwrap_or_else(|p| p.into_inner()) {
                    return;
                }
                std::thread::sleep(step);
                waited += step;
            }
            let _ = killpg(pgid, Signal::SIGKILL);
        });
    }

    fn screen(&self) -> Screen {
        self.screen.clone()
    }
}

impl Drop for PtyRun {
    fn drop(&mut self) {
        if !self.is_finished() {
            let _ = killpg(self.pgid, Signal::SIGHUP);
        }
    }
}
