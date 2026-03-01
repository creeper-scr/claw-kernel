//! Error types for claw-pal.
//!
//! Provides unified error handling across sandbox, IPC, and process management.

use std::fmt;

/// Sandbox-related errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxError {
    /// Failed to create sandbox.
    CreationFailed(String),
    /// Failed to apply sandbox restrictions.
    RestrictFailed(String),
    /// Sandbox restrictions already applied.
    AlreadyApplied,
    /// Sandbox feature not supported on this platform.
    NotSupported,
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SandboxError::CreationFailed(msg) => write!(f, "sandbox creation failed: {}", msg),
            SandboxError::RestrictFailed(msg) => write!(f, "sandbox restriction failed: {}", msg),
            SandboxError::AlreadyApplied => write!(f, "sandbox restrictions already applied"),
            SandboxError::NotSupported => write!(f, "sandbox not supported on this platform"),
        }
    }
}

impl std::error::Error for SandboxError {}

/// IPC-related errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcError {
    /// Connection refused.
    ConnectionRefused,
    /// Operation timed out.
    Timeout,
    /// Pipe broken.
    BrokenPipe,
    /// Invalid message format.
    InvalidMessage,
    /// Permission denied.
    PermissionDenied,
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpcError::ConnectionRefused => write!(f, "connection refused"),
            IpcError::Timeout => write!(f, "operation timed out"),
            IpcError::BrokenPipe => write!(f, "broken pipe"),
            IpcError::InvalidMessage => write!(f, "invalid message format"),
            IpcError::PermissionDenied => write!(f, "permission denied"),
        }
    }
}

impl std::error::Error for IpcError {}

/// Process-related errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessError {
    /// Failed to spawn process.
    SpawnFailed(String),
    /// Process not found.
    NotFound(u32),
    /// Permission denied.
    PermissionDenied,
    /// Invalid signal.
    InvalidSignal,
}

impl fmt::Display for ProcessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessError::SpawnFailed(msg) => write!(f, "process spawn failed: {}", msg),
            ProcessError::NotFound(pid) => write!(f, "process not found: {}", pid),
            ProcessError::PermissionDenied => write!(f, "permission denied"),
            ProcessError::InvalidSignal => write!(f, "invalid signal"),
        }
    }
}

impl std::error::Error for ProcessError {}

/// Unified error type for claw-pal operations.
#[derive(Debug)]
pub enum PalError {
    /// Sandbox error.
    Sandbox(SandboxError),
    /// IPC error.
    Ipc(IpcError),
    /// Process error.
    Process(ProcessError),
    /// Permission denied.
    PermissionDenied(String),
    /// IO error.
    Io(std::io::Error),
}

impl fmt::Display for PalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PalError::Sandbox(e) => write!(f, "sandbox error: {}", e),
            PalError::Ipc(e) => write!(f, "IPC error: {}", e),
            PalError::Process(e) => write!(f, "process error: {}", e),
            PalError::PermissionDenied(msg) => write!(f, "permission denied: {}", msg),
            PalError::Io(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for PalError {}

impl From<SandboxError> for PalError {
    fn from(err: SandboxError) -> Self {
        PalError::Sandbox(err)
    }
}

impl From<IpcError> for PalError {
    fn from(err: IpcError) -> Self {
        PalError::Ipc(err)
    }
}

impl From<ProcessError> for PalError {
    fn from(err: ProcessError) -> Self {
        PalError::Process(err)
    }
}

impl From<std::io::Error> for PalError {
    fn from(err: std::io::Error) -> Self {
        PalError::Io(err)
    }
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
    fn test_pal_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let pal_err: PalError = io_err.into();
        assert!(pal_err.to_string().contains("IO error"));
    }
}
