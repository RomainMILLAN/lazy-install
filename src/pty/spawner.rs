//! Starts an update in a pseudo-terminal.

use std::io::Read;
use std::sync::Arc;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use crate::catalog::{NotStartedReason, ScriptRef};
use crate::jobs::ports::{ActiveUpdate, RunEvents, UpdateSpawner};
use crate::script::command::CommandSpec;
use crate::script::contract::Function;
use crate::script::trust::TrustedScript;
use crate::script::validate::check_text_of;

use super::log::LogSink;
use super::run::PtyRun;
use super::screen::Screen;

#[derive(Default)]
pub struct PtySpawner;

impl PtySpawner {
    pub fn new() -> Self {
        PtySpawner
    }
}

impl UpdateSpawner for PtySpawner {
    fn spawn(
        &self,
        script: &ScriptRef,
        (rows, cols): (u16, u16),
        events: RunEvents,
    ) -> Result<Box<dyn ActiveUpdate>, NotStartedReason> {
        // The trust check and the contract text are re-done here, right before
        // the process exists: what was validated at registration may have
        // changed on disk since.
        let trusted = TrustedScript::verify(script.path())
            .map_err(|e| NotStartedReason::new(&e.to_string()))?;
        check_text_of(trusted.path()).map_err(|e| NotStartedReason::new(&e.to_string()))?;
        let spec = CommandSpec::run(&trusted, Function::Update, "xterm-256color");
        drop(trusted);

        let size = PtySize {
            rows: rows.max(1),
            cols: cols.max(1),
            pixel_width: 0,
            pixel_height: 0,
        };
        let pair = native_pty_system()
            .openpty(size)
            .map_err(|e| NotStartedReason::new(&format!("cannot open a pty: {e}")))?;

        let mut cmd = CommandBuilder::new(&spec.program);
        cmd.args(&spec.args);
        cmd.env_clear();
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        if let Some(home) = dirs::home_dir() {
            cmd.cwd(home);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| NotStartedReason::new(&format!("cannot start bash: {e}")))?;
        drop(pair.slave);

        let pid = child
            .process_id()
            .ok_or_else(|| NotStartedReason::new("the update process has no pid"))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| NotStartedReason::new(&e.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| NotStartedReason::new(&e.to_string()))?;

        let screen = Screen::new(size.rows, size.cols);
        let mut log = events
            .log_path
            .as_deref()
            .and_then(|p| match LogSink::open(p) {
                Ok(l) => Some(l),
                Err(e) => {
                    log::warn!("cannot open the run log {}: {e}", p.display());
                    None
                }
            });

        let reader_screen = screen.clone();
        let on_output = Arc::clone(&events.on_output);
        let reader_thread = std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        reader_screen.process(&buf[..n]);
                        if let Some(l) = log.as_mut() {
                            l.write(&buf[..n]);
                        }
                        on_output();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            log
        });

        Ok(Box::new(PtyRun::start(
            child,
            pid,
            pair.master,
            writer,
            screen,
            reader_thread,
            events.on_exit,
        )))
    }
}
