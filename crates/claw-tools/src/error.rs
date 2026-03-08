//! Error types for claw-tools.
//!
//! Provides unified error handling for tool registry operations including validation,
//! loading, execution, and file watching.

use std::path::PathBuf;

use thiserror::Error;

/// Validation error types for tool validation pipeline.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
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

#[derive(Debug, Error, Clone, PartialEq, Eq)]
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

#[derive(Debug, Error, Clone, PartialEq, Eq)]
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

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum WatchError {
    #[error("watch failed: {0}")]
    WatchFailed(String),
    #[error("path not found: {0}")]
    PathNotFound(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validation_error_display() {
        let err = ValidationError::SyntaxError {
            file: PathBuf::from("tool.lua"),
            message: "unexpected token".to_string(),
        };
        assert!(err.to_string().contains("syntax error"));
        assert!(err.to_string().contains("tool.lua"));

        let err = ValidationError::PermissionError {
            file: PathBuf::from("tool.lua"),
            issue: "unsafe call".to_string(),
        };
        assert!(err.to_string().contains("permission error"));

        let err = ValidationError::SchemaError {
            file: PathBuf::from("tool.lua"),
            details: vec!["missing field".to_string()],
        };
        assert!(err.to_string().contains("schema error"));

        let err = ValidationError::CompilationError {
            file: PathBuf::from("tool.lua"),
            stderr: "syntax error".to_string(),
        };
        assert!(err.to_string().contains("compilation error"));

        let err = ValidationError::TimeoutError {
            file: PathBuf::from("tool.lua"),
            operation: "validation".to_string(),
        };
        assert!(err.to_string().contains("timeout error"));
    }

    #[test]
    fn test_registry_error_display() {
        let err = RegistryError::ToolNotFound("my_tool".to_string());
        assert_eq!(err.to_string(), "tool not found: my_tool");

        let err = RegistryError::AlreadyExists("my_tool".to_string());
        assert_eq!(err.to_string(), "tool already registered: my_tool");

        let err = RegistryError::PermissionDenied {
            tool: "dangerous".to_string(),
            permission: "fs.write".to_string(),
        };
        assert!(err.to_string().contains("permission denied"));

        let err = RegistryError::Timeout(5000);
        assert_eq!(err.to_string(), "execution timed out after 5000ms");

        let err = RegistryError::ExecutionFailed("panic".to_string());
        assert_eq!(err.to_string(), "execution error: panic");
    }

    #[test]
    fn test_load_error_display() {
        let err = LoadError::FileNotFound("tool.lua".to_string());
        assert_eq!(err.to_string(), "file not found: tool.lua");

        let err = LoadError::ParseError("invalid syntax".to_string());
        assert_eq!(err.to_string(), "parse error: invalid syntax");

        let err = LoadError::CompileError("type mismatch".to_string());
        assert_eq!(err.to_string(), "compile error: type mismatch");

        let err = LoadError::Io("read failed".to_string());
        assert_eq!(err.to_string(), "IO error: read failed");
    }

    #[test]
    fn test_watch_error_display() {
        let err = WatchError::WatchFailed("inotify limit".to_string());
        assert_eq!(err.to_string(), "watch failed: inotify limit");

        let err = WatchError::PathNotFound("/missing".to_string());
        assert_eq!(err.to_string(), "path not found: /missing");
    }

    #[test]
    fn test_error_clone() {
        let err = RegistryError::ToolNotFound("tool".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }
}
