//! ProcessManager trait for cross-platform process management.
//!
//! Provides a unified interface for spawning, terminating, and monitoring processes
//! across Linux, macOS, and Windows.

use crate::error::ProcessError;
use crate::types::process::{ExitStatus, ProcessConfig, ProcessHandle, ProcessSignal};
use std::time::Duration;

/// Trait for managing processes in a cross-platform manner.
///
/// Implementations must handle platform-specific process management while providing
/// a consistent async interface. All methods are async and support cancellation.
#[allow(async_fn_in_trait)]
pub trait ProcessManager: Send + Sync {
    /// Spawn a new process with the given configuration.
    ///
    /// # Arguments
    /// * `config` - Process configuration (program, args, env, working directory)
    ///
    /// # Returns
    /// A `ProcessHandle` that can be used to interact with the spawned process.
    async fn spawn(&self, config: ProcessConfig) -> Result<ProcessHandle, ProcessError>;

    /// Terminate a process gracefully.
    ///
    /// Sends SIGTERM (Unix) or TerminateProcess (Windows) to the process.
    /// If the process does not exit within `grace_period`, it is forcefully killed.
    ///
    /// # Arguments
    /// * `handle` - Handle to the process to terminate
    /// * `grace_period` - Time to wait after SIGTERM before sending SIGKILL
    ///
    /// # Returns
    /// Ok(()) if the process was terminated, or an error if the operation failed.
    async fn terminate(
        &self,
        handle: ProcessHandle,
        grace_period: Duration,
    ) -> Result<(), ProcessError>;

    /// Kill a process immediately.
    ///
    /// Sends SIGKILL (Unix) or TerminateProcess (Windows) to the process.
    ///
    /// # Arguments
    /// * `handle` - Handle to the process to kill
    ///
    /// # Returns
    /// Ok(()) if the process was killed, or an error if the operation failed.
    async fn kill(&self, handle: ProcessHandle) -> Result<(), ProcessError>;

    /// Wait for a process to exit and return its exit status.
    ///
    /// Blocks until the process exits. If the process has already exited,
    /// returns immediately with the cached exit status.
    ///
    /// # Arguments
    /// * `handle` - Handle to the process to wait for
    ///
    /// # Returns
    /// The exit status of the process.
    async fn wait(&self, handle: ProcessHandle) -> Result<ExitStatus, ProcessError>;

    /// Send a signal to a process.
    ///
    /// Sends a cross-platform signal to the process. The signal is abstracted
    /// to avoid platform-specific signal numbers.
    ///
    /// # Arguments
    /// * `handle` - Handle to the process
    /// * `signal` - Signal to send (Term, Kill, Interrupt)
    ///
    /// # Returns
    /// Ok(()) if the signal was sent, or an error if the operation failed.
    async fn signal(
        &self,
        handle: ProcessHandle,
        signal: ProcessSignal,
    ) -> Result<(), ProcessError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Mock implementation of ProcessManager for testing.
    struct MockProcessManager;

    impl ProcessManager for MockProcessManager {
        async fn spawn(&self, _config: ProcessConfig) -> Result<ProcessHandle, ProcessError> {
            Ok(ProcessHandle {
                pid: 1234,
                name: "test_process".to_string(),
            })
        }

        async fn terminate(
            &self,
            _handle: ProcessHandle,
            _grace_period: Duration,
        ) -> Result<(), ProcessError> {
            Ok(())
        }

        async fn kill(&self, _handle: ProcessHandle) -> Result<(), ProcessError> {
            Ok(())
        }

        async fn wait(&self, _handle: ProcessHandle) -> Result<ExitStatus, ProcessError> {
            Ok(ExitStatus {
                code: Some(0),
                success: true,
            })
        }

        async fn signal(
            &self,
            _handle: ProcessHandle,
            _signal: ProcessSignal,
        ) -> Result<(), ProcessError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn test_mock_spawn() {
        let manager = MockProcessManager;
        let config = ProcessConfig {
            program: "echo".to_string(),
            args: vec!["hello".to_string()],
            env: HashMap::new(),
            working_dir: None,
        };

        let handle = manager.spawn(config).await.unwrap();
        assert_eq!(handle.pid, 1234);
        assert_eq!(handle.name, "test_process");
    }

    #[tokio::test]
    async fn test_mock_terminate() {
        let manager = MockProcessManager;
        let handle = ProcessHandle {
            pid: 1234,
            name: "test_process".to_string(),
        };

        manager
            .terminate(handle, Duration::from_secs(5))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_mock_kill() {
        let manager = MockProcessManager;
        let handle = ProcessHandle {
            pid: 1234,
            name: "test_process".to_string(),
        };

        manager.kill(handle).await.unwrap();
    }

    #[tokio::test]
    async fn test_mock_wait() {
        let manager = MockProcessManager;
        let handle = ProcessHandle {
            pid: 1234,
            name: "test_process".to_string(),
        };

        let status = manager.wait(handle).await.unwrap();
        assert!(status.success);
        assert_eq!(status.code, Some(0));
    }

    #[tokio::test]
    async fn test_mock_signal() {
        let manager = MockProcessManager;
        let handle = ProcessHandle {
            pid: 1234,
            name: "test_process".to_string(),
        };

        manager.signal(handle, ProcessSignal::Term).await.unwrap();
    }

    #[tokio::test]
    async fn test_process_handle_equality() {
        let handle1 = ProcessHandle {
            pid: 1234,
            name: "test".to_string(),
        };
        let handle2 = ProcessHandle {
            pid: 1234,
            name: "test".to_string(),
        };
        let handle3 = ProcessHandle {
            pid: 5678,
            name: "other".to_string(),
        };

        assert_eq!(handle1, handle2);
        assert_ne!(handle1, handle3);
    }

    #[test]
    fn test_process_signal_variants() {
        let _term = ProcessSignal::Term;
        let _kill = ProcessSignal::Kill;
        let _interrupt = ProcessSignal::Interrupt;
    }
}
