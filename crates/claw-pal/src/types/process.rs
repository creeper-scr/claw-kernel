//! Process-related types for claw-pal.
//!
//! Provides configuration and status types for process management.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration for spawning a new process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessConfig {
    /// Program name or path to execute.
    pub program: String,
    /// Command-line arguments.
    pub args: Vec<String>,
    /// Environment variables to set.
    pub env: HashMap<String, String>,
    /// Working directory for the process (None = inherit from parent).
    pub working_dir: Option<PathBuf>,
}

impl ProcessConfig {
    /// Create a new process configuration.
    pub fn new(program: String) -> Self {
        Self {
            program,
            args: Vec::new(),
            env: HashMap::new(),
            working_dir: None,
        }
    }

    /// Add a command-line argument.
    pub fn with_arg(mut self, arg: String) -> Self {
        self.args.push(arg);
        self
    }

    /// Add multiple command-line arguments.
    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args.extend(args);
        self
    }

    /// Set an environment variable.
    pub fn with_env(mut self, key: String, value: String) -> Self {
        self.env.insert(key, value);
        self
    }

    /// Set the working directory.
    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = Some(dir);
        self
    }
}

/// Handle to a running process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessHandle {
    /// Process ID (PID).
    pub pid: u32,
    /// Process name.
    pub name: String,
}

impl ProcessHandle {
    /// Create a new process handle.
    pub fn new(pid: u32, name: String) -> Self {
        Self { pid, name }
    }
}

/// Exit status of a process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStatus {
    /// Exit code (None if terminated by signal).
    pub code: Option<i32>,
    /// Whether the process exited successfully (code 0 or equivalent).
    pub success: bool,
}

impl ExitStatus {
    /// Create a successful exit status.
    pub fn success() -> Self {
        Self {
            code: Some(0),
            success: true,
        }
    }

    /// Create a failed exit status with the given code.
    pub fn failure(code: i32) -> Self {
        Self {
            code: Some(code),
            success: false,
        }
    }

    /// Create an exit status for a process terminated by signal.
    pub fn signal() -> Self {
        Self {
            code: None,
            success: false,
        }
    }
}

/// Cross-platform process signal.
///
/// Abstracts platform-specific signals to a common interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProcessSignal {
    /// Terminate signal (SIGTERM on Unix, TerminateProcess on Windows).
    Term,
    /// Kill signal (SIGKILL on Unix, TerminateProcess on Windows).
    Kill,
    /// Interrupt signal (SIGINT on Unix, Ctrl+C on Windows).
    Interrupt,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_config_new() {
        let config = ProcessConfig::new("echo".to_string());
        assert_eq!(config.program, "echo");
        assert!(config.args.is_empty());
        assert!(config.env.is_empty());
        assert_eq!(config.working_dir, None);
    }

    #[test]
    fn test_process_config_builder() {
        let config = ProcessConfig::new("echo".to_string())
            .with_arg("hello".to_string())
            .with_arg("world".to_string())
            .with_env("VAR".to_string(), "value".to_string())
            .with_working_dir(PathBuf::from("/tmp"));

        assert_eq!(config.program, "echo");
        assert_eq!(config.args, vec!["hello".to_string(), "world".to_string()]);
        assert_eq!(config.env.get("VAR"), Some(&"value".to_string()));
        assert_eq!(config.working_dir, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn test_process_config_with_args() {
        let config = ProcessConfig::new("echo".to_string()).with_args(vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
        ]);

        assert_eq!(
            config.args,
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }

    #[test]
    fn test_process_config_clone() {
        let config = ProcessConfig::new("echo".to_string()).with_arg("test".to_string());
        let cloned = config.clone();
        assert_eq!(config, cloned);
    }

    #[test]
    fn test_process_config_serialize() {
        let config = ProcessConfig::new("echo".to_string()).with_arg("hello".to_string());
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ProcessConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config, deserialized);
    }

    #[test]
    fn test_process_handle_new() {
        let handle = ProcessHandle::new(1234, "test".to_string());
        assert_eq!(handle.pid, 1234);
        assert_eq!(handle.name, "test");
    }

    #[test]
    fn test_process_handle_clone() {
        let handle = ProcessHandle::new(1234, "test".to_string());
        let cloned = handle.clone();
        assert_eq!(handle, cloned);
    }

    #[test]
    fn test_process_handle_serialize() {
        let handle = ProcessHandle::new(1234, "test".to_string());
        let json = serde_json::to_string(&handle).unwrap();
        let deserialized: ProcessHandle = serde_json::from_str(&json).unwrap();
        assert_eq!(handle, deserialized);
    }

    #[test]
    fn test_exit_status_success() {
        let status = ExitStatus::success();
        assert_eq!(status.code, Some(0));
        assert!(status.success);
    }

    #[test]
    fn test_exit_status_failure() {
        let status = ExitStatus::failure(1);
        assert_eq!(status.code, Some(1));
        assert!(!status.success);
    }

    #[test]
    fn test_exit_status_signal() {
        let status = ExitStatus::signal();
        assert_eq!(status.code, None);
        assert!(!status.success);
    }

    #[test]
    fn test_exit_status_clone() {
        let status = ExitStatus::success();
        let cloned = status.clone();
        assert_eq!(status, cloned);
    }

    #[test]
    fn test_exit_status_serialize() {
        let status = ExitStatus::failure(42);
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: ExitStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(status, deserialized);
    }

    #[test]
    fn test_process_signal_variants() {
        let _term = ProcessSignal::Term;
        let _kill = ProcessSignal::Kill;
        let _interrupt = ProcessSignal::Interrupt;
    }

    #[test]
    fn test_process_signal_clone() {
        let signal = ProcessSignal::Term;
        let cloned = signal;
        assert_eq!(signal, cloned);
    }

    #[test]
    fn test_process_signal_serialize() {
        let signal = ProcessSignal::Kill;
        let json = serde_json::to_string(&signal).unwrap();
        let deserialized: ProcessSignal = serde_json::from_str(&json).unwrap();
        assert_eq!(signal, deserialized);
    }
}
