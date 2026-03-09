//! Error types for claw-pal.
//!
//! Provides unified error handling across sandbox, IPC, and process management.

use thiserror::Error;

/// Sandbox-related errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// Failed to create sandbox.
    #[error("sandbox creation failed: {0}")]
    CreationFailed(String),
    /// Failed to apply sandbox restrictions.
    #[error("sandbox restriction failed: {0}")]
    RestrictFailed(String),
    /// Sandbox restrictions already applied.
    #[error("sandbox restrictions already applied")]
    AlreadyApplied,
    /// Sandbox feature not supported on this platform.
    #[error("sandbox not supported on this platform")]
    NotSupported,
}

/// IPC-related errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IpcError {
    /// Connection refused.
    #[error("connection refused")]
    ConnectionRefused,
    /// Operation timed out.
    #[error("operation timed out")]
    Timeout,
    /// Pipe broken.
    #[error("broken pipe")]
    BrokenPipe,
    /// Invalid message format.
    #[error("invalid message format")]
    InvalidMessage,
    /// Permission denied.
    #[error("permission denied")]
    PermissionDenied,
}

/// Process-related errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProcessError {
    /// Failed to spawn process.
    #[error("process spawn failed: {0}")]
    SpawnFailed(String),
    /// Failed to send signal to process.
    #[error("process signal failed: {0}")]
    SignalFailed(String),
    /// Process not found.
    #[error("process not found: {0}")]
    NotFound(u32),
    /// Permission denied.
    #[error("permission denied")]
    PermissionDenied,
    /// Invalid signal.
    #[error("invalid signal")]
    InvalidSignal,
}

/// Unified error type for claw-pal operations.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PalError {
    /// Sandbox error.
    #[error("sandbox error: {0}")]
    Sandbox(#[from] SandboxError),
    /// IPC error.
    #[error("IPC error: {0}")]
    Ipc(#[from] IpcError),
    /// Process error.
    #[error("process error: {0}")]
    Process(#[from] ProcessError),
    /// Permission denied.
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    /// IO error.
    #[error("IO error: {0}")]
    Io(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sandbox_error_display() {
        let err = SandboxError::CreationFailed("test".to_string());
        assert_eq!(err.to_string(), "sandbox creation failed: test");

        let err = SandboxError::RestrictFailed("test".to_string());
        assert_eq!(err.to_string(), "sandbox restriction failed: test");

        let err = SandboxError::AlreadyApplied;
        assert_eq!(err.to_string(), "sandbox restrictions already applied");

        let err = SandboxError::NotSupported;
        assert_eq!(err.to_string(), "sandbox not supported on this platform");
    }

    #[test]
    fn test_sandbox_error_clone() {
        let err = SandboxError::CreationFailed("test".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }

    #[test]
    fn test_ipc_error_display() {
        let err = IpcError::ConnectionRefused;
        assert_eq!(err.to_string(), "connection refused");

        let err = IpcError::Timeout;
        assert_eq!(err.to_string(), "operation timed out");

        let err = IpcError::BrokenPipe;
        assert_eq!(err.to_string(), "broken pipe");

        let err = IpcError::InvalidMessage;
        assert_eq!(err.to_string(), "invalid message format");

        let err = IpcError::PermissionDenied;
        assert_eq!(err.to_string(), "permission denied");
    }

    #[test]
    fn test_ipc_error_clone() {
        let err = IpcError::ConnectionRefused;
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }

    #[test]
    fn test_process_error_display() {
        let err = ProcessError::SpawnFailed("test".to_string());
        assert_eq!(err.to_string(), "process spawn failed: test");

        let err = ProcessError::SignalFailed("permission denied".to_string());
        assert_eq!(err.to_string(), "process signal failed: permission denied");

        let err = ProcessError::NotFound(1234);
        assert_eq!(err.to_string(), "process not found: 1234");

        let err = ProcessError::PermissionDenied;
        assert_eq!(err.to_string(), "permission denied");

        let err = ProcessError::InvalidSignal;
        assert_eq!(err.to_string(), "invalid signal");
    }

    #[test]
    fn test_process_error_clone() {
        let err = ProcessError::SpawnFailed("test".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }

    #[test]
    fn test_pal_error_from_sandbox() {
        let sandbox_err = SandboxError::CreationFailed("test".to_string());
        let pal_err: PalError = sandbox_err.into();
        assert_eq!(
            pal_err.to_string(),
            "sandbox error: sandbox creation failed: test"
        );
    }

    #[test]
    fn test_pal_error_from_ipc() {
        let ipc_err = IpcError::ConnectionRefused;
        let pal_err: PalError = ipc_err.into();
        assert_eq!(pal_err.to_string(), "IPC error: connection refused");
    }

    #[test]
    fn test_pal_error_from_process() {
        let process_err = ProcessError::NotFound(1234);
        let pal_err: PalError = process_err.into();
        assert_eq!(
            pal_err.to_string(),
            "process error: process not found: 1234"
        );
    }

    #[test]
    fn test_pal_error_permission_denied() {
        let pal_err = PalError::PermissionDenied("test".to_string());
        assert_eq!(pal_err.to_string(), "permission denied: test");
    }

    #[test]
    fn test_pal_error_io() {
        let pal_err = PalError::Io("file not found".to_string());
        assert!(pal_err.to_string().contains("IO error"));
    }
}
