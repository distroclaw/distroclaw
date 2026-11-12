//! Child processes the app starts itself (the page renderer, the PDF reader): reading their
//! output without blocking, and ending them together with everything they started.

use std::io::Read;
use std::process::Child;
#[cfg(windows)]
use std::process::Command;
use std::thread::JoinHandle;
#[cfg(unix)]
use std::time::{Duration, Instant};

/// Raw bytes read from each pipe; the rest is drained and dropped.
pub(crate) const MAX_PIPE_BYTES: usize = 4 * 1024 * 1024;
/// After SIGTERM, how long the group gets before SIGKILL.
#[cfg(unix)]
const GRACE: Duration = Duration::from_millis(800);

/// Windows: start a console program without opening a console window.
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A program that comes with Windows, from its system folder: a bare name would be looked up in
/// the app's own folder first.
#[cfg(windows)]
pub(crate) fn system_tool(name: &str) -> std::path::PathBuf {
    let root = std::env::var_os("SystemRoot").map_or_else(|| r"C:\Windows".into(), std::path::PathBuf::from);
    root.join("System32").join(name)
}

/// Reads a pipe to its end on a thread, keeping the first `MAX_PIPE_BYTES`.
pub(crate) fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut kept = Vec::new();
        let Some(mut pipe) = pipe else { return kept };
        let mut buf = [0u8; 8192];
        while let Ok(n) = pipe.read(&mut buf) {
            if n == 0 {
                break;
            }
            if kept.len() < MAX_PIPE_BYTES {
                kept.extend_from_slice(&buf[..n.min(MAX_PIPE_BYTES - kept.len())]);
            }
        }
        kept
    })
}

/// Ends the child and everything it started: TERM, a short grace, then KILL. The child must
/// have been started in its own process group.
pub(crate) fn kill_group(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        // SAFETY: plain libc calls on a process group this process created.
        unsafe { libc::killpg(pid, libc::SIGTERM) };
        let until = Instant::now() + GRACE;
        while Instant::now() < until {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        unsafe { libc::killpg(pid, libc::SIGKILL) };
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new(system_tool("taskkill.exe"))
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }
    let _ = child.kill();
}
