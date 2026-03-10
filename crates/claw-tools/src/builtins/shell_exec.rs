//! Built-in shell execution tool.
//!
//! This tool is NOT registered by default due to security risk.
//! It must be explicitly registered via `registry.register(Box::new(ShellExecTool::new()))`.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;

use crate::traits::Tool;
use crate::types::{
    FsPermissions, NetworkPermissions, PermissionSet, SubprocessPolicy, ToolContext, ToolError,
    ToolResult, ToolSchema,
};

/// Shell execution tool. NOT registered by default.
///
/// Executes arbitrary shell commands via `sh -c`. Enforces a configurable
/// timeout (default: 30s). Must be explicitly registered if needed.
///
/// # Security Warning
///
/// This tool allows arbitrary code execution. Only register it in trusted
/// environments with appropriate sandboxing.
pub struct ShellExecTool;

impl ShellExecTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ShellExecTool {
    fn default() -> Self {
        Self::new()
    }
}

static SHELL_EXEC_SCHEMA: OnceLock<ToolSchema> = OnceLock::new();
static SHELL_EXEC_PERMS: OnceLock<PermissionSet> = OnceLock::new();

#[async_trait]
impl Tool for ShellExecTool {
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command via sh -c. Security risk: must be explicitly enabled."
    }

    fn schema(&self) -> &ToolSchema {
        SHELL_EXEC_SCHEMA.get_or_init(|| {
            ToolSchema::new(
                "shell_exec",
                "Execute a shell command via sh -c. Security risk: must be explicitly enabled.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to execute"
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "default": 30,
                            "description": "Maximum execution time in seconds"
                        }
                    },
                    "required": ["command"]
                }),
            )
        })
    }

    fn permissions(&self) -> &PermissionSet {
        SHELL_EXEC_PERMS.get_or_init(|| PermissionSet {
            filesystem: FsPermissions::none(),
            network: NetworkPermissions::none(),
            subprocess: SubprocessPolicy::Allowed,
        })
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let command = match args["command"].as_str() {
            Some(c) => c.to_string(),
            None => {
                return ToolResult::err(
                    ToolError::invalid_args("'command' parameter is required"),
                    0,
                );
            }
        };
        let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);
        let start = std::time::Instant::now();

        let result = tokio::time::timeout(
            Duration::from_secs(timeout_secs),
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&command)
                .output(),
        )
        .await;

        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            Err(_) => ToolResult::err(
                ToolError::internal(format!(
                    "Command timed out after {timeout_secs}s: {command}"
                )),
                elapsed,
            ),
            Ok(Err(e)) => ToolResult::err(
                ToolError::internal(format!("Failed to execute command '{command}': {e}")),
                elapsed,
            ),
            Ok(Ok(output)) => ToolResult::ok(
                serde_json::json!({
                    "exit_code": output.status.code(),
                    "stdout": String::from_utf8_lossy(&output.stdout),
                    "stderr": String::from_utf8_lossy(&output.stderr),
                }),
                elapsed,
            ),
        }
    }
}
