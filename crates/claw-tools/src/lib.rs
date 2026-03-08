//! Tool registry, protocol, and hot-loading.

pub mod audit;
pub mod error;
pub mod hot_loader;
pub mod hot_reload;
pub mod registry;
pub mod traits;
pub mod types;

pub use audit::{AuditEvent, AuditLogConfig, AuditLogWriter, AuditLogWriterHandle};
pub use error::{LoadError, RegistryError, ValidationError, WatchError};
#[allow(deprecated)]
pub use hot_loader::HotLoader;
pub use registry::ToolRegistry;
pub use traits::Tool;
pub use types::{
    FsPermissions, HotLoadingConfig, LogEntry, NetworkPermissions, PermissionSet, ScriptLanguage,
    SubprocessPolicy, ToolContext, ToolError, ToolErrorCode, ToolMeta, ToolResult, ToolSchema,
    ToolSource,
};
