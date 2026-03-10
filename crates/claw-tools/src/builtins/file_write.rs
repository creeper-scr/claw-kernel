//! Built-in tool for writing files to the filesystem.

use std::path::Path;
use std::sync::OnceLock;

use async_trait::async_trait;

use crate::traits::Tool;
use crate::types::{
    FsPermissions, NetworkPermissions, PermissionSet, SubprocessPolicy, ToolContext, ToolError,
    ToolResult, ToolSchema,
};

/// Built-in tool for writing content to a file.
///
/// For security, optionally restricts writes to a configured allowed directory.
/// By default, will not overwrite existing files unless `overwrite: true` is set.
pub struct FileWriteTool {
    allowed_dir: Option<std::path::PathBuf>,
}

impl FileWriteTool {
    pub fn new() -> Self {
        Self { allowed_dir: None }
    }

    /// Restrict file writes to the given directory (and its subdirectories).
    pub fn with_allowed_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        self.allowed_dir = Some(dir.into());
        self
    }
}

impl Default for FileWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

static FILE_WRITE_SCHEMA: OnceLock<ToolSchema> = OnceLock::new();
static FILE_WRITE_PERMS: OnceLock<PermissionSet> = OnceLock::new();

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Will not overwrite by default. Restricted to allowed directories when configured."
    }

    fn schema(&self) -> &ToolSchema {
        FILE_WRITE_SCHEMA.get_or_init(|| {
            ToolSchema::new(
                "file_write",
                "Write content to a file. Will not overwrite by default. Restricted to allowed directories when configured.",
                serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to write to"
                        },
                        "content": {
                            "type": "string",
                            "description": "Content to write"
                        },
                        "overwrite": {
                            "type": "boolean",
                            "default": false,
                            "description": "Allow overwriting an existing file"
                        }
                    },
                    "required": ["path", "content"]
                }),
            )
        })
    }

    fn permissions(&self) -> &PermissionSet {
        FILE_WRITE_PERMS.get_or_init(|| PermissionSet {
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
        let content = match args["content"].as_str() {
            Some(c) => c.to_string(),
            None => {
                return ToolResult::err(
                    ToolError::invalid_args("'content' parameter is required"),
                    0,
                );
            }
        };
        let overwrite = args["overwrite"].as_bool().unwrap_or(false);

        let start = std::time::Instant::now();
        let path = Path::new(&path_str);

        // Security: check against allowed_dir if set (check parent directory)
        if let Some(allowed) = &self.allowed_dir {
            let check_path = if path.exists() {
                path.to_path_buf()
            } else {
                // File doesn't exist yet — check the parent directory
                path.parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| path.to_path_buf())
            };

            if check_path.exists() {
                match (check_path.canonicalize(), allowed.canonicalize()) {
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
                            ToolError::invalid_args(format!(
                                "Cannot resolve path '{}': {e}",
                                path_str
                            )),
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
        }

        // Overwrite guard
        if !overwrite && path.exists() {
            return ToolResult::err(
                ToolError::permission_denied(format!(
                    "File '{}' already exists. Set overwrite=true to replace it.",
                    path_str
                )),
                start.elapsed().as_millis() as u64,
            );
        }

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return ToolResult::err(
                    ToolError::internal(format!(
                        "Failed to create parent directories for '{}': {e}",
                        path_str
                    )),
                    start.elapsed().as_millis() as u64,
                );
            }
        }

        let bytes_written = content.len();
        match tokio::fs::write(path, &content).await {
            Ok(()) => ToolResult::ok(
                serde_json::json!({
                    "success": true,
                    "path": path_str,
                    "bytes_written": bytes_written
                }),
                start.elapsed().as_millis() as u64,
            ),
            Err(e) => ToolResult::err(
                ToolError::internal(format!("Failed to write file '{}': {e}", path_str)),
                start.elapsed().as_millis() as u64,
            ),
        }
    }
}
