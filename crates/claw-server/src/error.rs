//! Error types for claw-server.
//!
//! Provides unified error handling for the KernelServer JSON-RPC 2.0 server.

use thiserror::Error;

/// Server-related errors.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ServerError {
    /// IPC error from claw-pal.
    #[error("IPC error: {0}")]
    Ipc(#[from] claw_pal::error::IpcError),
    /// Serialization/deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),
    /// Session not found.
    #[error("session not found: {0}")]
    SessionNotFound(String),
    /// Provider error from claw-provider.
    #[error("provider error: {0}")]
    Provider(#[from] claw_provider::error::ProviderError),
    /// Agent error.
    #[error("agent error: {0}")]
    Agent(String),
    /// Server is shutting down.
    #[error("server shutting down")]
    Shutdown,
    /// Maximum number of sessions reached.
    #[error("max sessions reached: {max}")]
    MaxSessionsReached {
        /// The maximum number of sessions allowed.
        max: usize,
    },
}

impl ServerError {
    /// Returns the JSON-RPC 2.0 error code for this error.
    pub fn error_code(&self) -> i32 {
        #[allow(unreachable_patterns)]
        match self {
            ServerError::Serialization(_) => -32700,   // Parse error
            ServerError::Ipc(_) => -32600,             // Invalid Request
            ServerError::SessionNotFound(_) => -32000, // Session not found
            ServerError::MaxSessionsReached { .. } => -32001, // Max sessions reached
            ServerError::Provider(_) => -32002,        // Provider error
            ServerError::Agent(_) => -32003,           // Agent error
            ServerError::Shutdown => -32600,           // Invalid Request (server unavailable)
            _ => -32600, // Invalid Request (catch-all for future variants)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_error_ipc() {
        let ipc_err = claw_pal::error::IpcError::ConnectionRefused;
        let err = ServerError::Ipc(ipc_err);
        assert!(err.to_string().contains("IPC error"));
        assert_eq!(err.error_code(), -32600);
    }

    #[test]
    fn test_server_error_serialization() {
        let err = ServerError::Serialization("invalid json".to_string());
        assert_eq!(err.to_string(), "serialization error: invalid json");
        assert_eq!(err.error_code(), -32700);
    }

    #[test]
    fn test_server_error_session_not_found() {
        let err = ServerError::SessionNotFound("session-123".to_string());
        assert_eq!(err.to_string(), "session not found: session-123");
        assert_eq!(err.error_code(), -32000);
    }

    #[test]
    fn test_server_error_agent() {
        let err = ServerError::Agent("loop failed".to_string());
        assert_eq!(err.to_string(), "agent error: loop failed");
        assert_eq!(err.error_code(), -32003);
    }

    #[test]
    fn test_server_error_shutdown() {
        let err = ServerError::Shutdown;
        assert_eq!(err.to_string(), "server shutting down");
        assert_eq!(err.error_code(), -32600);
    }

    #[test]
    fn test_server_error_max_sessions_reached() {
        let err = ServerError::MaxSessionsReached { max: 100 };
        assert_eq!(err.to_string(), "max sessions reached: 100");
        assert_eq!(err.error_code(), -32001);
    }
}
