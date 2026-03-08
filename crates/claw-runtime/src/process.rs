//! Higher-level process management wrapper for `claw-runtime`.
//!
//! Provides [`ManagedProcess`], a convenience wrapper that pairs a
//! [`ProcessHandle`] with its owning [`TokioProcessManager`] so callers can
//! `wait` or `kill` a process without holding a reference to the manager
//! separately.

use claw_pal::traits::ProcessManager as _;
use claw_pal::{ExitStatus, ProcessHandle, TokioProcessManager};
use std::sync::Arc;
use thiserror::Error;

// ─── ProcessError ─────────────────────────────────────────────────────────────

/// Error type returned by [`ManagedProcess`] operations.
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("process operation failed: {0}")]
    PalError(String),
}

// ─── ManagedProcess ───────────────────────────────────────────────────────────

/// A running OS process paired with its process manager.
///
/// Obtained by calling [`claw_pal::TokioProcessManager::spawn`] and wrapping the
/// result. Provides ergonomic [`wait`](ManagedProcess::wait) and
/// [`kill`](ManagedProcess::kill) methods without requiring the caller to hold a
/// separate reference to the manager.
pub struct ManagedProcess {
    handle: ProcessHandle,
    manager: Arc<TokioProcessManager>,
}

impl ManagedProcess {
    /// Wrap a raw [`ProcessHandle`] and its owning manager.
    pub fn new(handle: ProcessHandle, manager: Arc<TokioProcessManager>) -> Self {
        Self { handle, manager }
    }

    /// Return a reference to the underlying [`ProcessHandle`].
    pub fn handle(&self) -> &ProcessHandle {
        &self.handle
    }

    /// Wait for the process to exit and return its [`ExitStatus`].
    pub async fn wait(&self) -> Result<ExitStatus, ProcessError> {
        self.manager
            .wait(self.handle.clone())
            .await
            .map_err(|e| ProcessError::PalError(e.to_string()))
    }

    /// Kill the process immediately (SIGKILL / TerminateProcess).
    pub async fn kill(&self) -> Result<(), ProcessError> {
        self.manager
            .kill(self.handle.clone())
            .await
            .map_err(|e| ProcessError::PalError(e.to_string()))
    }
}
