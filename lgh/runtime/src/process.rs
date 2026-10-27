use std::process::Stdio;
use std::time::Duration;
use tokio::process::{Child, Command};

use lgh_core::LghError;

const TERMINATE_GRACE: Duration = Duration::from_secs(3);

pub struct ManagedProcess {
    child: Child,
    pid: u32,
}

impl ManagedProcess {
    pub async fn spawn(cmd: &str, args: &[&str], envs: &[(&str, &str)]) -> Result<Self, LghError> {
        let mut command = Command::new(cmd);
        command
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        for (k, v) in envs {
            command.env(k, v);
        }

        // Harness displays have no input-method server, so IM variables
        // inherited from the user session only make XIM/GTK clients stall
        // (xterm can block ~5s in XIM init on XMODIFIERS=@im=<missing>).
        for var in ["XMODIFIERS", "GTK_IM_MODULE", "QT_IM_MODULE"] {
            command.env_remove(var);
        }

        let child = command
            .spawn()
            .map_err(|e| LghError::ProcessSpawnFailed(e.to_string()))?;

        let pid = child.id().unwrap_or(0);
        Ok(Self { child, pid })
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Terminate the process gracefully: SIGTERM first so apps release
    /// their MIT-SHM segments and Xvfb cleans up its socket and lock file,
    /// then SIGKILL after the grace period if it ignores the signal.
    pub async fn kill(&mut self) -> Result<(), LghError> {
        // tokio's Child::kill sends SIGKILL; deliver SIGTERM via the
        // `kill` binary (the crate forbids unsafe, so no libc/nix here).
        if self.pid > 0 {
            let _ = tokio::process::Command::new("kill")
                .args(["-TERM", &self.pid.to_string()])
                .status()
                .await;
            let deadline = tokio::time::Instant::now() + TERMINATE_GRACE;
            while tokio::time::Instant::now() < deadline {
                // try_wait reaps the child and reports its exit. Polling an
                // external `kill -0` here cannot see an unreaped zombie and
                // would always burn the full grace period even when the
                // process exited on SIGTERM immediately.
                match self.child.try_wait() {
                    Ok(Some(_status)) => return Ok(()),
                    Ok(None) => {}
                    Err(_) => break,
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }
        self.child
            .kill()
            .await
            .map_err(|e| LghError::ProcessKillFailed(e.to_string()))?;
        Ok(())
    }
}
