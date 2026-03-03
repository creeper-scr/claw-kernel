use std::time::Duration;

use async_trait::async_trait;

use crate::types::{PermissionSet, ToolContext, ToolResult, ToolSchema};

/// Core trait for tool implementations.
///
/// Tools can be written in Rust (native) or script languages (via bridge).
/// The `execute` method receives JSON arguments and a context containing
/// permissions and agent ID.
///
/// # Examples
///
/// Implementing a simple native tool:
///
/// ```rust
/// use claw_tools::{Tool, ToolSchema, PermissionSet, ToolContext, ToolResult};
/// use claw_tools::{ToolError, ToolErrorCode};
/// use async_trait::async_trait;
/// use std::time::Duration;
///
/// /// A simple tool that echoes back the input
/// struct EchoTool {
///     schema: ToolSchema,
///     permissions: PermissionSet,
/// }
///
/// impl EchoTool {
///     fn new() -> Self {
///         Self {
///             schema: ToolSchema::new(
///                 "echo",
///                 "Echoes back the input message",
///                 serde_json::json!({
///                     "type": "object",
///                     "properties": {
///                         "message": {
///                             "type": "string",
///                             "description": "The message to echo"
///                         }
///                     },
///                     "required": ["message"]
///                 }),
///             ),
///             permissions: PermissionSet::minimal(),
///         }
///     }
/// }
///
/// #[async_trait]
/// impl Tool for EchoTool {
///     fn name(&self) -> &str {
///         "echo"
///     }
///
///     fn description(&self) -> &str {
///         "Echoes back the input message"
///     }
///
///     fn schema(&self) -> &ToolSchema {
///         &self.schema
///     }
///
///     fn permissions(&self) -> &PermissionSet {
///         &self.permissions
///     }
///
///     // Optional: override the default 30s timeout
///     fn timeout(&self) -> Duration {
///         Duration::from_secs(10)
///     }
///
///     async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult {
///         // Validate required argument
///         let message = match args.get("message") {
///             Some(m) => m.as_str().unwrap_or(""),
///             None => {
///                 return ToolResult::err(
///                     ToolError::invalid_args("Missing 'message' argument"),
///                     0
///                 );
///             }
///         };
///
///         // Build response with agent context
///         let output = serde_json::json!({
///             "echo": message,
///             "agent_id": ctx.agent_id,
///         });
///
///         ToolResult::ok(output, 1)
///     }
/// }
///
/// # async fn example() {
/// let tool = EchoTool::new();
/// let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
/// let args = serde_json::json!({"message": "Hello!"});
///
/// let result = tool.execute(args, &ctx).await;
/// assert!(result.success);
/// assert_eq!(result.output.unwrap()["echo"], "Hello!");
/// # }
/// ```
///
/// Using the tool registry:
///
/// ```rust
/// use claw_tools::ToolRegistry;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create a registry for managing tools
/// let registry = ToolRegistry::new();
///
/// // Tools can be registered by passing a boxed Tool implementation
/// // registry.register(Box::new(EchoTool::new()))?;
///
/// // Retrieve a tool by name
/// // let tool = registry.get("echo");
///
/// // List all available tool names
/// let tool_names = registry.tool_names();
///
/// // Get count of registered tools
/// let count = registry.tool_count();
/// # Ok(())
/// # }
/// ```
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique tool name (snake_case).
    fn name(&self) -> &str;

    /// Human-readable description shown to the LLM.
    fn description(&self) -> &str;

    /// JSON Schema for input parameters.
    fn schema(&self) -> &ToolSchema;

    /// Permissions required by this tool.
    fn permissions(&self) -> &PermissionSet;

    /// Maximum execution time. Default: 30 seconds.
    fn timeout(&self) -> Duration {
        Duration::from_secs(30)
    }

    /// Execute the tool with the given JSON arguments.
    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult;
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{PermissionSet, ToolContext, ToolResult, ToolSchema};
    use async_trait::async_trait;

    struct EchoTool {
        schema: ToolSchema,
        perms: PermissionSet,
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echo input as output"
        }
        fn schema(&self) -> &ToolSchema {
            &self.schema
        }
        fn permissions(&self) -> &PermissionSet {
            &self.perms
        }
        async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::ok(args, 1)
        }
    }

    fn make_echo_tool() -> EchoTool {
        EchoTool {
            schema: ToolSchema::new("echo", "Echo input as output", serde_json::json!({})),
            perms: PermissionSet::minimal(),
        }
    }

    #[tokio::test]
    async fn test_echo_tool_execute() {
        let tool = make_echo_tool();
        let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
        let args = serde_json::json!({"msg": "hello"});
        let result = tool.execute(args.clone(), &ctx).await;
        assert!(result.success);
        assert_eq!(result.output.as_ref().unwrap(), &args);
        assert_eq!(result.duration_ms, 1);
    }

    #[tokio::test]
    async fn test_echo_tool_default_timeout_30s() {
        let tool = make_echo_tool();
        assert_eq!(tool.timeout(), Duration::from_secs(30));
    }

    #[tokio::test]
    async fn test_tool_name_description() {
        let tool = make_echo_tool();
        assert_eq!(tool.name(), "echo");
        assert_eq!(tool.description(), "Echo input as output");
    }
}
