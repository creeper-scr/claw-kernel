//! Error types for claw-runtime.
//!
//! Provides unified error handling for runtime operations including agent management,
//! IPC communication, event bus, and shutdown coordination.

use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    #[error("agent not found: {0}")]
    AgentNotFound(String),
    #[error("agent already exists: {0}")]
    AgentAlreadyExists(String),
    #[error("IPC error: {0}")]
    IpcError(String),
    #[error("event bus error: {0}")]
    EventBusError(String),
    #[error("shutdown in progress")]
    ShuttingDown,
    #[error("operation timed out")]
    Timeout,
    #[error("process error: {0}")]
    ProcessError(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime_error_display() {
        let err = RuntimeError::AgentNotFound("agent1".to_string());
        assert_eq!(err.to_string(), "agent not found: agent1");

        let err = RuntimeError::AgentAlreadyExists("agent2".to_string());
        assert_eq!(err.to_string(), "agent already exists: agent2");

        let err = RuntimeError::IpcError("connection lost".to_string());
        assert_eq!(err.to_string(), "IPC error: connection lost");

        let err = RuntimeError::EventBusError("channel full".to_string());
        assert_eq!(err.to_string(), "event bus error: channel full");

        let err = RuntimeError::ShuttingDown;
        assert_eq!(err.to_string(), "shutdown in progress");

        let err = RuntimeError::Timeout;
        assert_eq!(err.to_string(), "operation timed out");

        let err = RuntimeError::ProcessError("spawn failed".to_string());
        assert_eq!(err.to_string(), "process error: spawn failed");
    }

    #[test]
    fn test_runtime_error_clone() {
        let err = RuntimeError::AgentNotFound("agent1".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }
}
