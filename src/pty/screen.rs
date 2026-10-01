//! The emulated screen of a run.

use std::sync::{Arc, Mutex, MutexGuard};

const SCROLLBACK: usize = 5000;

/// A vt100 screen shared between the reader thread and the renderer.
///
/// The lock never leaves this module: callers get the screen for the duration
/// of a closure, so no one can hold it across a frame by accident.
#[derive(Clone)]
pub struct Screen(Arc<Mutex<vt100::Parser>>);

impl Screen {
    pub fn new(rows: u16, cols: u16) -> Self {
        Screen(Arc::new(Mutex::new(vt100::Parser::new(
            rows.max(1),
            cols.max(1),
            SCROLLBACK,
        ))))
    }

    fn lock(&self) -> MutexGuard<'_, vt100::Parser> {
        // A panic while holding the lock leaves a parser that is still usable.
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn with_screen<R>(&self, f: impl FnOnce(&vt100::Screen) -> R) -> R {
        f(self.lock().screen())
    }

    pub(crate) fn process(&self, bytes: &[u8]) {
        self.lock().process(bytes);
    }

    pub(crate) fn set_size(&self, rows: u16, cols: u16) {
        self.lock().screen_mut().set_size(rows.max(1), cols.max(1));
    }

    /// Scrolls the view into the scrollback; 0 is the live screen.
    pub fn scroll(&self, delta: isize) {
        let mut p = self.lock();
        let cur = p.screen().scrollback() as isize;
        let next = (cur + delta).max(0) as usize;
        p.screen_mut().set_scrollback(next);
    }

    pub fn reset_scroll(&self) {
        self.lock().screen_mut().set_scrollback(0);
    }

    /// Whether the program asked for application cursor keys (`ESC O A`).
    pub fn application_cursor(&self) -> bool {
        self.with_screen(|s| s.application_cursor())
    }

    /// The last non-empty line on screen, for the "looks like a password
    /// prompt" heuristic.
    pub fn last_line(&self) -> String {
        self.with_screen(|s| {
            s.contents()
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string()
        })
    }
}

/// `password`, `passphrase`, `[sudo]`: a prompt is waiting for keys.
pub fn looks_like_password_prompt(line: &str) -> bool {
    let l = line.to_lowercase();
    l.contains("password") || l.contains("passphrase") || l.contains("[sudo]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processes_and_reads_back() {
        let s = Screen::new(5, 20);
        s.process(b"hello\r\n[sudo] password for me: ");
        assert!(looks_like_password_prompt(&s.last_line()));
        assert!(s.with_screen(|sc| sc.contents()).contains("hello"));
    }
}
