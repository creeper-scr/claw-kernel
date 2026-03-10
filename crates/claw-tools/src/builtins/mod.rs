//! Built-in tools provided by claw-tools when the `builtins` feature is enabled.

mod file_read;
mod file_write;
mod shell_exec;
mod web_fetch;

pub use file_read::FileReadTool;
pub use file_write::FileWriteTool;
pub use shell_exec::ShellExecTool;
pub use web_fetch::WebFetchTool;

use crate::{RegistryError, ToolRegistry};

/// Register all built-in tools into the given registry.
///
/// Note: `ShellExecTool` is NOT registered by default due to security risk.
/// Call `registry.register(Box::new(ShellExecTool::new()))` explicitly if needed.
pub fn register_all_builtins(registry: &ToolRegistry) -> Result<(), RegistryError> {
    registry.register(Box::new(WebFetchTool::new()))?;
    registry.register(Box::new(FileReadTool::new()))?;
    registry.register(Box::new(FileWriteTool::new()))?;
    Ok(())
}
