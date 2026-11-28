//! Stateless servers left running by a launcher that died (Linux, ADR 0031).
//!
//! On Windows a kill-on-close Job Object ends them with the launcher. Linux
//! has no safe equivalent here (`PR_SET_PDEATHSIG` fires when the spawning
//! *thread* ends, and the async runtime's threads come and go), so each
//! `terminate` process leaves its pid in `<log>.pid`, and the next start ends
//! the one it names when it still runs that same program. Servers with state
//! never need it: worldserver stops cleanly when its stdin closes, MySQL is
//! adopted.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// `<console log>.pid`.
pub(crate) fn pid_file(log_file: &Path) -> PathBuf {
    log_file.with_extension("pid")
}

/// Records the pid of a process just started.
pub(crate) fn record(log_file: &Path, pid: u32) -> std::io::Result<()> {
    std::fs::write(pid_file(log_file), format!("{pid}\n"))
}

/// Forgets it once it exited.
pub(crate) fn forget(log_file: &Path) {
    let _ = std::fs::remove_file(pid_file(log_file));
}

/// Ends the process a previous run left behind, if it still runs `program`.
/// A pid reused by another program is left alone.
pub(crate) async fn end_leftover(log_file: &Path, program: &Path) {
    let Some(pid) = std::fs::read_to_string(pid_file(log_file))
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok())
    else {
        return;
    };
    if runs(pid, program) {
        let _ = tokio::process::Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .status()
            .await;
        let started = Instant::now();
        while runs(pid, program) && started.elapsed() < Duration::from_secs(5) {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    forget(log_file);
}

/// Whether process `pid` runs `program` (`/proc/<pid>/exe`).
fn runs(pid: u32, program: &Path) -> bool {
    let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
        return false;
    };
    let program = std::fs::canonicalize(program).unwrap_or_else(|_| program.to_path_buf());
    exe == program
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_leftover_running_the_program_is_ended() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("authserver");
        std::fs::copy("/bin/sleep", &program).unwrap();
        let log = dir.path().join("authserver.console.log");
        let mut child = std::process::Command::new(&program)
            .arg("60")
            .spawn()
            .unwrap();
        record(&log, child.id()).unwrap();

        // Another program never matches: a reused pid is left alone.
        assert!(!runs(child.id(), Path::new("/bin/true")));
        end_leftover(&log, &program).await;
        let status = child.wait().unwrap();
        assert!(!status.success());
        assert!(!pid_file(&log).exists());
    }
}
