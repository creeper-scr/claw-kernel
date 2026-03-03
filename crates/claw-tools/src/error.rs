use std::path::PathBuf;

use thiserror::Error;

/// Validation error types for tool validation pipeline.
#[derive(Debug, Error)]
pub enum ValidationError {
    /// Syntax error in the tool script.
    #[error("syntax error in {file}: {message}")]
    SyntaxError { file: PathBuf, message: String },
    /// Permission audit failed.
    #[error("permission error in {file}: {issue}")]
    PermissionError { file: PathBuf, issue: String },
    /// Schema validation failed.
    #[error("schema error in {file}: {details:?}")]
    SchemaError { file: PathBuf, details: Vec<String> },
    /// Compilation error in sandbox.
    #[error("compilation error in {file}: {stderr}")]
    CompilationError { file: PathBuf, stderr: String },
    /// Operation timed out.
    #[error("timeout error in {file}: {operation} timed out")]
    TimeoutError { file: PathBuf, operation: String },
}

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
