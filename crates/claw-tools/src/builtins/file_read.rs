//! Built-in tool for reading files from the filesystem.

use std::path::Path;
use std::sync::OnceLock;

use async_trait::async_trait;

use crate::traits::Tool;
use crate::types::{
    FsPermissions, NetworkPermissions, PermissionSet, SubprocessPolicy, ToolContext, ToolError,
    ToolResult, ToolSchema,
};

/// Built-in tool for reading files.
///
/// For security, optionally restricts reads to a configured allowed directory.
pub struct FileReadTool {
    allowed_dir: Option<std::path::PathBuf>,
}

impl FileReadTool {
    pub fn new() -> Self {
        Self { allowed_dir: None }
    }

    /// Restrict file reads to the given directory (and its subdirectories).
    pub fn with_allowed_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.allowed_dir = Some(dir.into());
        self
    }
}

impl Default for FileReadTool {
    fn default() -> Self {
        Self::new()
    }
}

static FILE_READ_SCHEMA: OnceLock<ToolSchema> = OnceLock::new();
static FILE_READ_PERMS: OnceLock<PermissionSet> = OnceLock::new();

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Restricted to allowed directories when configured."
    }

    fn schema(&self) -> &ToolSchema {
        FILE_READ_SCHEMA.get_or_init(|| {
            ToolSchema::new(
                "file_read",
                "Read the contents of a file. Restricted to allowed directories when configured.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to read"
                        }
                    },
                    "required": ["path"]
                }),
            )
        })
    }

    fn permissions(&self) -> &PermissionSet {
        FILE_READ_PERMS.get_or_init(|| PermissionSet {
            filesystem: FsPermissions::none(),
            network: NetworkPermissions::none(),
            subprocess: SubprocessPolicy::Denied,
        })
    }

    async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let path_str = match args["path"].as_str() {
            Some(p) => p.to_string(),
            None => {
                return ToolResult::err(
                    ToolError::invalid_args("'path' parameter is required"),
                    0,
                );
            }
        };

        let start = std::time::Instant::now();
        let path = Path::new(&path_str);

        // Security: check against allowed_dir if set
        if let Some(allowed) = &self.allowed_dir {
            match (path.canonicalize(), allowed.canonicalize()) {
                (Ok(canonical), Ok(allowed_canonical)) => {
                    if !canonical.starts_with(&allowed_canonical) {
                        return ToolResult::err(
                            ToolError::permission_denied(format!(
                                "Access denied: '{}' is outside allowed directory '{}'",
                                path_str,
                                allowed.display()
                            )),
                            start.elapsed().as_millis() as u64,
                        );
                    }
                }
                (Err(e), _) => {
                    return ToolResult::err(
                        ToolError::invalid_args(format!("Cannot resolve path '{}': {e}", path_str)),
                        start.elapsed().as_millis() as u64,
                    );
                }
                (_, Err(e)) => {
                    return ToolResult::err(
                        ToolError::internal(format!(
                            "Cannot resolve allowed directory '{}': {e}",
                            allowed.display()
                        )),
                        start.elapsed().as_millis() as u64,
                    );
                }
            }
        }

        match tokio::fs::read_to_string(path).await {
            Ok(content) => ToolResult::ok(
                serde_json::json!({ "content": content, "path": path_str }),
                start.elapsed().as_millis() as u64,
            ),
            Err(e) => ToolResult::err(
                ToolError::internal(format!("Failed to read file '{}': {e}", path_str)),
                start.elapsed().as_millis() as u64,
            ),
        }
    }
}
