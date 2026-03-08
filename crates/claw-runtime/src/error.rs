use thiserror::Error;

#[derive(Debug, Error, Clone)]
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
