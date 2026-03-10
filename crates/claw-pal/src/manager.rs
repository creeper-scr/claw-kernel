//! TokioProcessManager - Tokio-based implementation of ProcessManager.
//!
//! Manages spawned child processes using a DashMap keyed by PID.
//! Each Child is wrapped in a Mutex because Child: Send but not Sync.

use crate::error::ProcessError;
use crate::traits::ProcessManager;
use crate::types::process::{ExitStatus, ProcessConfig, ProcessHandle, ProcessSignal};
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Child;
use tokio::sync::Mutex;

/// Tokio-based process manager.
///
/// Internally stores spawned processes in a `DashMap<u32, Mutex<Child>>`.
/// Each `Child` is wrapped in a `Mutex` because `tokio::process::Child`
/// is `Send` but not `Sync`.
pub struct TokioProcessManager {
    children: Arc<DashMap<u32, Mutex<Child>>>,
}

impl TokioProcessManager {
    /// Create a new, empty `TokioProcessManager`.
    pub fn new() -> Self {
        Self {
            children: Arc::new(DashMap::new()),
        }
    }
}

impl Default for TokioProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Send a Unix signal to a process by PID.
///
/// Supported on Linux and macOS only.
#[cfg(unix)]
fn send_unix_signal(pid: u32, signal: ProcessSignal) -> Result<(), ProcessError> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    let nix_signal = match signal {
        ProcessSignal::Term => Signal::SIGTERM,
        ProcessSignal::Interrupt => Signal::SIGINT,
        ProcessSignal::Kill => Signal::SIGKILL,
    };
    kill(Pid::from_raw(pid as i32), nix_signal)
        .map_err(|e| ProcessError::SignalFailed(e.to_string()))
}

/// Signal stub for Windows.
///
/// On Windows SIGTERM/SIGINT are not natively supported; callers should
/// use `kill()` instead.  This function always succeeds so that the
/// `signal()` method can fall through to `kill()` for the `Kill` variant.
#[cfg(windows)]
fn send_windows_signal(_pid: u32, _signal: ProcessSignal) -> Result<(), ProcessError> {
    // Windows does not expose SIGTERM/SIGINT via a standard API.
    // The caller is expected to call child.kill() for ProcessSignal::Kill.
    Ok(())
}

#[async_trait::async_trait]
impl ProcessManager for TokioProcessManager {
    /// Spawn a new process with the given configuration.
    async fn spawn(&self, config: ProcessConfig) -> Result<ProcessHandle, ProcessError> {
        let mut cmd = tokio::process::Command::new(&config.program);
        cmd.args(&config.args);
        cmd.envs(&config.env);

        if let Some(ref dir) = config.working_dir {
            cmd.current_dir(dir);
        }

        // Do not kill the child automatically on drop; we manage lifetimes
        // explicitly via the DashMap.
        cmd.kill_on_drop(false);

        let child = cmd
            .spawn()
            .map_err(|e| ProcessError::SpawnFailed(e.to_string()))?;

        let pid = child
            .id()
            .ok_or_else(|| ProcessError::SpawnFailed("failed to obtain PID".to_string()))?;

        let name = config.program.clone();

        // Guard against PID reuse: check if this PID is already tracked.
        if self.children.contains_key(&pid) {
            tracing::warn!(pid = pid, "PID already tracked; possible PID reuse or stale entry");
            // Check whether the old process has already exited.
            let still_running = {
                let old_entry = self.children.get_mut(&pid).unwrap();
                let mut old_child = old_entry.value().try_lock();
                match old_child {
                    Ok(ref mut child) => child.try_wait().ok().flatten().is_none(),
                    Err(_) => true, // Mutex is locked — assume process is still running.
                }
            };
            if still_running {
                // Old process is still running — refuse the insert.
                return Err(ProcessError::SpawnFailed(
                    format!("PID {} collision: previous process still running", pid)
                ));
            }
            self.children.remove(&pid);
        }

        self.children.insert(pid, Mutex::new(child));

        Ok(ProcessHandle { pid, name })
    }

    /// Gracefully terminate a process.
    ///
    /// Sends SIGTERM (Unix) then waits up to `grace_period`.  If the
    /// process does not exit in time, sends SIGKILL and waits again.
    async fn terminate(
        &self,
        handle: ProcessHandle,
        grace_period: Duration,
    ) -> Result<(), ProcessError> {
        let pid = handle.pid;

        // Deliver soft signal first (best-effort; ignore errors here so we
        // always proceed to the timeout / kill path).
        #[cfg(unix)]
        let _ = send_unix_signal(pid, ProcessSignal::Term);

        // On Windows there is no native SIGTERM, so we skip straight to the
        // timed wait and then TerminateProcess.
        #[cfg(windows)]
        let _ = send_windows_signal(pid, ProcessSignal::Term);

        // Poll every 10 ms to detect early exit, then force-kill on timeout.
        let deadline = tokio::time::Instant::now() + grace_period;
        loop {
            {
                let entry = self
                    .children
                    .get_mut(&pid)
                    .ok_or(ProcessError::NotFound(pid))?;
                let mut child = entry.lock().await;
                if let Ok(Some(_)) = child.try_wait() {
                    drop(child);
                    drop(entry);
                    self.children.remove(&pid);
                    return Ok(());
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        // Grace period expired — force-kill.
        {
            let entry = self
                .children
                .get(&pid)
                .ok_or(ProcessError::NotFound(pid))?;
            let mut child = entry.lock().await;
            let _ = child.kill().await;
            let _ = child.wait().await;
            drop(child);
            drop(entry);
        }
        self.children.remove(&pid);
        Ok(())
    }

    /// Kill a process immediately (SIGKILL / TerminateProcess).
    async fn kill(&self, handle: ProcessHandle) -> Result<(), ProcessError> {
        let pid = handle.pid;

        let entry = self.children.get(&pid).ok_or(ProcessError::NotFound(pid))?;

        let mut child = entry.lock().await;
        child
            .kill()
            .await
            .map_err(|e| ProcessError::SignalFailed(e.to_string()))?;

        // Reap the zombie.
        let _ = child.wait().await;

        drop(child);
        drop(entry);

        self.children.remove(&pid);

        Ok(())
    }

    /// Wait for a process to exit and return its exit status.
    async fn wait(&self, handle: ProcessHandle) -> Result<ExitStatus, ProcessError> {
        let pid = handle.pid;

        let entry = self.children.get(&pid).ok_or(ProcessError::NotFound(pid))?;

        let mut child = entry.lock().await;

        let status = child
            .wait()
            .await
            .map_err(|e| ProcessError::SignalFailed(e.to_string()))?;

        drop(child);
        drop(entry);

        // Remove the now-exited child from the map.
        self.children.remove(&pid);

        Ok(ExitStatus {
            code: status.code(),
            success: status.success(),
        })
    }

    /// Send a signal to a process.
    ///
    /// `ProcessSignal::Kill` always calls `kill()`.
    /// `ProcessSignal::Term` and `ProcessSignal::Interrupt` use the
    /// platform-native signal mechanism on Unix; on Windows they fall back
    /// to `kill()`.
    async fn signal(
        &self,
        handle: ProcessHandle,
        signal: ProcessSignal,
    ) -> Result<(), ProcessError> {
        match signal {
            ProcessSignal::Kill => {
                self.kill(handle).await?;
            }
            #[cfg(unix)]
            ProcessSignal::Term | ProcessSignal::Interrupt => {
                let pid = handle.pid;
                if !self.children.contains_key(&pid) {
                    return Err(ProcessError::NotFound(pid));
                }
                send_unix_signal(pid, signal)?;
            }
            #[cfg(windows)]
            ProcessSignal::Term | ProcessSignal::Interrupt => {
                // Fall back to kill on Windows.
                self.kill(handle).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ----------------------------------------------------------------
    // Helper: build a ProcessConfig for a short-lived command
    // ----------------------------------------------------------------
    fn echo_config() -> ProcessConfig {
        ProcessConfig {
            program: "echo".to_string(),
            args: vec!["hello".to_string()],
            env: HashMap::new(),
            working_dir: None,
        }
    }

    #[cfg(unix)]
    fn sleep_config(secs: u64) -> ProcessConfig {
        ProcessConfig {
            program: "sleep".to_string(),
            args: vec![secs.to_string()],
            env: HashMap::new(),
            working_dir: None,
        }
    }

    #[cfg(windows)]
    fn sleep_config(secs: u64) -> ProcessConfig {
        // Windows equivalent: ping loops with 1-second intervals
        ProcessConfig {
            program: "ping".to_string(),
            args: vec!["-n".to_string(), secs.to_string(), "127.0.0.1".to_string()],
            env: HashMap::new(),
            working_dir: None,
        }
    }

    // ----------------------------------------------------------------
    // 1. test_new_creates_empty_manager
    // ----------------------------------------------------------------
    #[test]
    fn test_new_creates_empty_manager() {
        let manager = TokioProcessManager::new();
        assert_eq!(manager.children.len(), 0);
    }

    // ----------------------------------------------------------------
    // 2. test_spawn_echo_command
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_spawn_echo_command() {
        let manager = TokioProcessManager::new();
        let handle = manager.spawn(echo_config()).await.unwrap();
        assert!(handle.pid > 0, "pid must be positive");
        assert_eq!(handle.name, "echo");
        // Reap the process to avoid zombies in the test runner.
        let _ = manager.wait(handle).await;
    }

    // ----------------------------------------------------------------
    // 3. test_spawn_nonexistent_program_fails
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_spawn_nonexistent_program_fails() {
        let manager = TokioProcessManager::new();
        let config = ProcessConfig {
            program: "nonexistent_xxx_program_12345".to_string(),
            args: vec![],
            env: HashMap::new(),
            working_dir: None,
        };
        let result = manager.spawn(config).await;
        assert!(
            result.is_err(),
            "spawning a nonexistent program should fail"
        );
        match result.unwrap_err() {
            ProcessError::SpawnFailed(_) => {}
            other => panic!("expected SpawnFailed, got {:?}", other),
        }
    }

    // ----------------------------------------------------------------
    // 4. test_kill_running_process
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_kill_running_process() {
        let manager = TokioProcessManager::new();
        let handle = manager.spawn(sleep_config(60)).await.unwrap();
        let result = manager.kill(handle).await;
        assert!(result.is_ok(), "kill should succeed: {:?}", result);
    }

    // ----------------------------------------------------------------
    // 5. test_wait_process_exits_naturally
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_wait_process_exits_naturally() {
        let manager = TokioProcessManager::new();
        let handle = manager.spawn(echo_config()).await.unwrap();
        let status = manager.wait(handle).await.unwrap();
        assert!(status.success, "echo should exit with code 0");
        assert_eq!(status.code, Some(0));
    }

    // ----------------------------------------------------------------
    // 6. test_terminate_fast_process
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_terminate_fast_process() {
        let manager = TokioProcessManager::new();
        let handle = manager.spawn(echo_config()).await.unwrap();
        // Give it a generous grace period; echo exits almost instantly.
        let result = manager.terminate(handle, Duration::from_secs(5)).await;
        assert!(result.is_ok(), "terminate should succeed: {:?}", result);
    }

    // ----------------------------------------------------------------
    // 7. test_spawn_with_args
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_spawn_with_args() {
        let manager = TokioProcessManager::new();
        let config = ProcessConfig {
            program: "echo".to_string(),
            args: vec!["hello".to_string(), "world".to_string()],
            env: HashMap::new(),
            working_dir: None,
        };
        let handle = manager.spawn(config).await.unwrap();
        assert!(handle.pid > 0);
        let status = manager.wait(handle).await.unwrap();
        assert!(status.success);
    }

    // ----------------------------------------------------------------
    // 8. test_spawn_with_env
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_spawn_with_env() {
        let manager = TokioProcessManager::new();
        let mut env = HashMap::new();
        env.insert("CLAW_TEST_VAR".to_string(), "hello_from_claw".to_string());

        #[cfg(unix)]
        let config = ProcessConfig {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), "echo $CLAW_TEST_VAR".to_string()],
            env,
            working_dir: None,
        };
        #[cfg(windows)]
        let config = ProcessConfig {
            program: "cmd".to_string(),
            args: vec!["/C".to_string(), "echo %CLAW_TEST_VAR%".to_string()],
            env,
            working_dir: None,
        };

        let handle = manager.spawn(config).await.unwrap();
        let status = manager.wait(handle).await.unwrap();
        assert!(status.success);
    }

    // ----------------------------------------------------------------
    // 9. test_spawn_with_working_dir
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_spawn_with_working_dir() {
        let manager = TokioProcessManager::new();
        let tmp = std::env::temp_dir();

        #[cfg(unix)]
        let config = ProcessConfig {
            program: "pwd".to_string(),
            args: vec![],
            env: HashMap::new(),
            working_dir: Some(tmp.clone()),
        };
        #[cfg(windows)]
        let config = ProcessConfig {
            program: "cmd".to_string(),
            args: vec!["/C".to_string(), "cd".to_string()],
            env: HashMap::new(),
            working_dir: Some(tmp.clone()),
        };

        let handle = manager.spawn(config).await.unwrap();
        let status = manager.wait(handle).await.unwrap();
        assert!(status.success);
    }

    // ----------------------------------------------------------------
    // 10. test_signal_kill
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_signal_kill() {
        let manager = TokioProcessManager::new();
        let handle = manager.spawn(sleep_config(60)).await.unwrap();
        let result = manager.signal(handle, ProcessSignal::Kill).await;
        assert!(result.is_ok(), "signal(Kill) should succeed: {:?}", result);
    }

    // ----------------------------------------------------------------
    // 11. test_manager_default
    // ----------------------------------------------------------------
    #[test]
    fn test_manager_default() {
        let manager = TokioProcessManager::default();
        assert_eq!(manager.children.len(), 0);
    }

    // ----------------------------------------------------------------
    // 12. test_manager_send_sync - compile-time assertion
    // ----------------------------------------------------------------
    #[test]
    fn test_manager_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TokioProcessManager>();
    }

    // ----------------------------------------------------------------
    // 13. test_terminate_long_running_process (grace period timeout path)
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_terminate_long_running_process() {
        let manager = TokioProcessManager::new();
        let handle = manager.spawn(sleep_config(60)).await.unwrap();
        // Use a very short grace period so the SIGKILL path is exercised.
        let result = manager.terminate(handle, Duration::from_millis(50)).await;
        assert!(
            result.is_ok(),
            "terminate with short grace should force-kill: {:?}",
            result
        );
    }

    // ----------------------------------------------------------------
    // 14. test_wait_returns_not_found_for_unknown_pid
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_wait_returns_not_found_for_unknown_pid() {
        let manager = TokioProcessManager::new();
        let fake_handle = ProcessHandle {
            pid: 999_999_999,
            name: "ghost".to_string(),
        };
        let result = manager.wait(fake_handle).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProcessError::NotFound(999_999_999) => {}
            other => panic!("expected NotFound(999_999_999), got {:?}", other),
        }
    }

    // ----------------------------------------------------------------
    // 15. test_kill_returns_not_found_for_unknown_pid
    // ----------------------------------------------------------------
    #[tokio::test]
    async fn test_kill_returns_not_found_for_unknown_pid() {
        let manager = TokioProcessManager::new();
        let fake_handle = ProcessHandle {
            pid: 999_999_998,
            name: "ghost".to_string(),
        };
        let result = manager.kill(fake_handle).await;
        assert!(result.is_err());
        match result.unwrap_err() {
            ProcessError::NotFound(999_999_998) => {}
            other => panic!("expected NotFound(999_999_998), got {:?}", other),
        }
    }
}
