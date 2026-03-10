//! Tool registry, protocol, and hot-loading.
//!
//! This crate provides a registry for managing tools that can be invoked
//! by the agent loop. Tools can be native Rust implementations or scripts.
//!
//! # Main Types
//!
//! - [`ToolRegistry`] - Thread-safe registry for tools
//! - [`Tool`] - Trait for implementing custom tools
//! - [`ToolSchema`] - JSON Schema for tool parameters
//! - [`PermissionSet`] - Permissions required by tools
//! - [`ToolContext`] - Execution context passed to tools
//! - [`ToolResult`] - Result type for tool execution
//! - [`HotReloadProcessor`] - Hot-reloading for tool scripts (requires ScriptEngine)
//!
//! # Example
//!
//! ```rust
//! use claw_tools::{ToolRegistry, Tool, ToolSchema, PermissionSet, ToolContext, ToolResult};
//! use async_trait::async_trait;
//!
//! struct EchoTool;
//!
//! #[async_trait]
//! impl Tool for EchoTool {
//!     fn name(&self) -> &str { "echo" }
//!     fn description(&self) -> &str { "Echoes back the input" }
//!     fn schema(&self) -> &ToolSchema {
//!         static SCHEMA: std::sync::OnceLock<ToolSchema> = std::sync::OnceLock::new();
//!         SCHEMA.get_or_init(|| ToolSchema::new("echo", "Echoes back the input", serde_json::json!({
//!             "type": "object",
//!             "properties": {
//!                 "message": { "type": "string" }
//!             },
//!             "required": ["message"]
//!         })))
//!     }
//!     fn permissions(&self) -> &PermissionSet {
//!         static PERMS: std::sync::OnceLock<PermissionSet> = std::sync::OnceLock::new();
//!         PERMS.get_or_init(PermissionSet::minimal)
//!     }
//!     async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
//!         ToolResult::ok(args, 1)
//!     }
//! }
//!
//! # fn main() {
//! let registry = ToolRegistry::new();
//! registry.register(Box::new(EchoTool)).expect("registration succeeds");
//! assert_eq!(registry.tool_count(), 1);
//! assert!(registry.tool_names().contains(&"echo".to_string()));
//! # }
//! ```

pub mod audit;
#[cfg(feature = "builtins")]
pub mod builtins;
pub mod error;
pub mod hot_reload;
pub mod registry;
pub mod traits;
pub mod types;

pub use audit::{
    AuditEvent, AuditLogConfig, AuditLogWriter, AuditLogWriterHandle, AuditStore,
    SecurityAuditEventRepr, ToolsAuditSink,
};
pub use error::{RegistryError, ValidationError};
pub use registry::{PowerKeyVerify, ToolRegistry};
pub use traits::{NoopToolEventPublisher, ScriptToolCompiler, Tool, ToolEventPublisher};
pub use types::{
    FsPermissions, HotLoadingConfig, LoadedToolMeta, LoadError, LogEntry, NetworkPermissions,
    PermissionSet, RegistryExecutionMode, ScriptLanguage, SubprocessPolicy, ToolContext, ToolError,
    ToolErrorCode, ToolMeta, ToolResult, ToolSchema, ToolSource, WatchError,
};
