//! Error types for claw-script.
//!
//! Provides unified error handling for script compilation and execution across
//! supported script engines (Lua, V8).

use thiserror::Error;

/// Errors during script compilation.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CompileError {
    #[error("syntax error: {0}")]
    Syntax(String),
    #[error("unsupported language: {0}")]
    UnsupportedLanguage(String),
}

/// All errors that can arise from script execution.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ScriptError {
    #[error("compile error: {0}")]
    Compile(#[from] CompileError),
    #[error("runtime error: {0}")]
    Runtime(String),
    #[error("type error: {0}")]
    TypeError(String),
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("timeout")]
    Timeout,
    #[error("engine not available: {0}")]
    EngineUnavailable(String),
    #[error("recursion limit exceeded (max {0} levels)")]
    RecursionLimitExceeded(u32),
    #[error("setup error: {0}")]
    Setup(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compile_error_display() {
        let e = CompileError::Syntax("unexpected token".to_string());
        assert_eq!(e.to_string(), "syntax error: unexpected token");

        let e2 = CompileError::UnsupportedLanguage("deno".to_string());
        assert_eq!(e2.to_string(), "unsupported language: deno");
    }

    #[test]
    fn test_script_error_from_compile_error() {
        let compile_err = CompileError::Syntax("bad syntax".to_string());
        let script_err: ScriptError = compile_err.into();
        assert!(matches!(script_err, ScriptError::Compile(_)));
        assert!(script_err.to_string().contains("compile error"));
    }

    #[test]
    fn test_script_error_variants_display() {
        let e1 = ScriptError::Runtime("panic".to_string());
        assert_eq!(e1.to_string(), "runtime error: panic");

        let e2 = ScriptError::TypeError("expected number".to_string());
        assert_eq!(e2.to_string(), "type error: expected number");

        let e3 = ScriptError::PermissionDenied("fs access".to_string());
        assert_eq!(e3.to_string(), "permission denied: fs access");

        let e4 = ScriptError::Timeout;
        assert_eq!(e4.to_string(), "timeout");

        let e5 = ScriptError::EngineUnavailable("v8".to_string());
        assert_eq!(e5.to_string(), "engine not available: v8");

        let e6 = ScriptError::RecursionLimitExceeded(32);
        assert_eq!(e6.to_string(), "recursion limit exceeded (max 32 levels)");
    }

    #[test]
    fn test_error_clone() {
        let err = CompileError::Syntax("bad".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);

        let err = ScriptError::Runtime("panic".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }
}
