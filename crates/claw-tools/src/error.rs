use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("tool not found: {0}")]
    ToolNotFound(String),
    #[error("tool already registered: {0}")]
    AlreadyExists(String),
    #[error("permission denied: tool '{tool}' requires {permission}")]
    PermissionDenied { tool: String, permission: String },
    #[error("execution timed out after {0}ms")]
    Timeout(u64),
    #[error("execution error: {0}")]
    ExecutionFailed(String),
}

#[derive(Debug, Error)]
pub enum LoadError {
    #[error("file not found: {0}")]
    FileNotFound(String),
    #[error("parse error: {0}")]
    ParseError(String),
    #[error("compile error: {0}")]
    CompileError(String),
    #[error("IO error: {0}")]
    Io(String),
}

#[derive(Debug, Error)]
pub enum WatchError {
    #[error("watch failed: {0}")]
    WatchFailed(String),
    #[error("path not found: {0}")]
    PathNotFound(String),
}
