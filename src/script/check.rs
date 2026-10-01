//! Runs `needs_update` and turns what it did into a `CheckOutcome`.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::fcntl::OFlag;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
use nix::sys::signal::{killpg, Signal};
use nix::sys::wait::{waitid, Id, WaitPidFlag, WaitStatus};
use nix::unistd::{pipe2, Pid};

use crate::catalog::{CheckOutcome, DisplayText, ScriptRef};
use crate::jobs::ports::CheckRunner;

use super::command::CommandSpec;
use super::contract::{decide, Function};
use super::trust::TrustedScript;
use super::validate::check_text_of;

/// Bytes kept per stream; the rest is read and thrown away so the child never
/// blocks on a full pipe.
const STREAM_CAP: usize = 64 * 1024;
/// After bash has exited, how long a child left in the background may keep a
/// pipe open before we stop reading. A valid answer is not turned into a
/// timeout by a stray `sleep &`.
const DRAIN: Duration = Duration::from_millis(200);

pub struct BashCheckRunner {
    timeout: Duration,
}

impl Default for BashCheckRunner {
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

impl BashCheckRunner {
    pub fn new(timeout: Duration) -> Self {
        BashCheckRunner { timeout }
    }

    /// The longest one `run` can take: the check up to its timeout (the drain
    /// window is inside it), plus a margin for what surrounds it — the trust
    /// check, reading the script text (no `bash -n` here: that runs only at
    /// registration), spawning and reaping. Derived from this instance's
    /// timeout, so the timeout has one source of truth.
    pub fn worst_case(&self) -> Duration {
        self.timeout + Self::MARGIN
    }

    const MARGIN: Duration = Duration::from_secs(5);
}

impl CheckRunner for BashCheckRunner {
    fn run(&self, script: &ScriptRef) -> CheckOutcome {
        let trusted = match TrustedScript::verify(script.path()) {
            Ok(t) => t,
            Err(e) => return CheckOutcome::Invalid(DisplayText::message(&e.to_string())),
        };
        if let Err(e) = check_text_of(trusted.path()) {
            return CheckOutcome::Invalid(DisplayText::message(&e.to_string()));
        }
        match run_check(trusted, self.timeout) {
            Ok(outcome) => outcome,
            Err(e) => CheckOutcome::Errored(DisplayText::message(&format!("cannot run bash: {e}"))),
        }
    }
}

fn nonce() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Consumes the proof: it was established for this run and no other.
fn run_check(script: TrustedScript, timeout: Duration) -> std::io::Result<CheckOutcome> {
    let spec = CommandSpec::run(&script, Function::NeedsUpdate, "dumb");
    let nonce = nonce()?;

    // Both pipes are CLOEXEC: a check running in parallel must not inherit the
    // write end of ours (it would hold it open and fake a timeout), nor anything
    // else. Only the dup2 below, in this child, clears the flag.
    let (token_r, token_w) = pipe2(OFlag::O_CLOEXEC)?;
    let (nonce_r, nonce_w) = pipe2(OFlag::O_CLOEXEC)?;
    let token_w_raw = token_w.as_raw_fd();
    let nonce_r_raw = nonce_r.as_raw_fd();

    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .env_clear()
        .envs(spec.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    // SAFETY: only async-signal-safe calls (fcntl, dup2, close) between fork and
    // exec. The pipes are first moved above 10 so that dup2(…, 3) cannot clobber
    // the other one if it happened to be fd 3 or 4.
    unsafe {
        cmd.pre_exec(move || {
            use nix::libc;
            let t = libc::fcntl(token_w_raw, libc::F_DUPFD, 10);
            let n = libc::fcntl(nonce_r_raw, libc::F_DUPFD, 10);
            if t < 0 || n < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::dup2(t, 3) < 0 || libc::dup2(n, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::close(t);
            libc::close(n);
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    drop(token_w);
    drop(nonce_r);

    // The nonce travels on fd 4, never in the environment.
    {
        let mut w = File::from(nonce_w);
        let _ = w.write_all(format!("{nonce}\n").as_bytes());
    }

    let pgid = Pid::from_raw(child.id() as i32);
    let collected = collect(&mut child, token_r, timeout);

    // Bash has been observed (not reaped) as exited, or we timed out. Killing the
    // group while bash is still a zombie means its pgid cannot have been reused.
    match killpg(pgid, Signal::SIGKILL) {
        Ok(()) | Err(Errno::ESRCH) | Err(Errno::EPERM) => {}
        Err(e) => log::warn!("killpg: {e}"),
    }
    let status = child.wait()?;

    if collected.timed_out {
        return Ok(CheckOutcome::Errored(DisplayText::message(&format!(
            "timeout: needs_update did not answer within {}s",
            timeout.as_secs()
        ))));
    }
    let code = status.code();
    Ok(decide(
        code,
        &collected.token,
        &nonce,
        Function::NeedsUpdate,
        &collected.stderr,
    ))
}

struct Collected {
    token: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

struct Stream {
    file: Option<File>,
    buf: Vec<u8>,
}

impl Stream {
    fn new(fd: Option<OwnedFd>) -> Self {
        Stream {
            file: fd.map(File::from),
            buf: Vec::new(),
        }
    }

    fn read_once(&mut self) {
        let Some(f) = self.file.as_mut() else { return };
        let mut chunk = [0u8; 8192];
        match f.read(&mut chunk) {
            Ok(0) => self.file = None,
            Ok(n) => {
                let room = STREAM_CAP.saturating_sub(self.buf.len());
                self.buf.extend_from_slice(&chunk[..n.min(room)]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => self.file = None,
        }
    }
}

/// One reader for the three descriptors, on `poll`, with a deadline. Returns
/// once bash has exited and the pipes are drained (or DRAIN has passed), or at
/// the deadline. Dropping the streams closes our ends: nothing is left behind.
fn collect(child: &mut Child, token_r: OwnedFd, timeout: Duration) -> Collected {
    let deadline = Instant::now() + timeout;
    let mut streams = [
        Stream::new(child.stdout.take().map(OwnedFd::from)),
        Stream::new(child.stderr.take().map(OwnedFd::from)),
        Stream::new(Some(token_r)),
    ];
    let pid = Pid::from_raw(child.id() as i32);
    let mut exited_at: Option<Instant> = None;

    loop {
        let now = Instant::now();
        if now >= deadline {
            return Collected {
                token: std::mem::take(&mut streams[2].buf),
                stderr: std::mem::take(&mut streams[1].buf),
                timed_out: true,
            };
        }
        if exited_at.is_none() {
            // WNOWAIT: observe the exit without reaping, so the pgid stays ours.
            if let Ok(status) = waitid(
                Id::Pid(pid),
                WaitPidFlag::WEXITED | WaitPidFlag::WNOWAIT | WaitPidFlag::WNOHANG,
            ) {
                if !matches!(status, WaitStatus::StillAlive) {
                    exited_at = Some(now);
                }
            }
        }
        let all_closed = streams.iter().all(|s| s.file.is_none());
        if let Some(at) = exited_at {
            if all_closed || now >= at + DRAIN {
                break;
            }
        }

        let open: Vec<usize> = (0..3).filter(|i| streams[*i].file.is_some()).collect();
        if open.is_empty() {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let ready: Vec<bool> = {
            let mut fds: Vec<PollFd> = open
                .iter()
                .map(|i| {
                    PollFd::new(
                        streams[*i].file.as_ref().unwrap().as_fd(),
                        PollFlags::POLLIN,
                    )
                })
                .collect();
            match poll(&mut fds, PollTimeout::from(50u16)) {
                Ok(_) => fds
                    .iter()
                    .map(|f| f.revents().is_some_and(|r| !r.is_empty()))
                    .collect(),
                Err(_) => vec![false; open.len()],
            }
        };
        for (k, i) in open.iter().enumerate() {
            if ready[k] {
                streams[*i].read_once();
            }
        }
    }

    Collected {
        token: std::mem::take(&mut streams[2].buf),
        stderr: std::mem::take(&mut streams[1].buf),
        timed_out: false,
    }
}

#[cfg(test)]
mod worst_case_tests {
    use super::*;

    #[test]
    fn worst_case_follows_the_instance_timeout() {
        assert!(BashCheckRunner::default().worst_case() > Duration::from_secs(30));
        let slow = BashCheckRunner::new(Duration::from_secs(60));
        assert!(slow.worst_case() > Duration::from_secs(60));
    }
}
