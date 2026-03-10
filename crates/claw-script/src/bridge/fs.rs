//! Lua-Rust filesystem bridge with sandbox path validation.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use claw_tools::audit::{AuditEvent, AuditLogWriterHandle};
use mlua::{Lua, Result as LuaResult, UserData, UserDataMethods};

/// 脚本可读取的最大文件大小（8 MiB）。
const MAX_READ_FILE_SIZE: u64 = 8 * 1024 * 1024;
/// list_dir 返回的最大目录条目数。
const MAX_LIST_DIR_ENTRIES: usize = 10_000;

/// Filesystem bridge exposing sandboxed file operations to Lua.
///
/// All paths are validated against `allowed_paths` — any access outside
/// these directories is rejected with a permission error.
pub struct FsBridge {
    /// Set of allowed base directories for filesystem access.
    allowed_paths: HashSet<PathBuf>,
    /// Base directory for resolving relative paths.
    base_dir: PathBuf,
    /// Optional audit log writer for recording FS operations.
    audit_handle: Option<AuditLogWriterHandle>,
    /// Agent ID used in audit events.
    agent_id: String,
}

impl FsBridge {
    /// Create a new FsBridge with the given allowed paths and base directory.
    pub fn new(
        allowed_paths: impl IntoIterator<Item = PathBuf>,
        base_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            allowed_paths: allowed_paths.into_iter().collect(),
            base_dir: base_dir.into(),
            audit_handle: None,
            agent_id: String::new(),
        }
    }

    /// Attach an audit log writer and agent ID to this bridge.
    pub fn with_audit(mut self, handle: AuditLogWriterHandle, agent_id: impl Into<String>) -> Self {
        self.audit_handle = Some(handle);
        self.agent_id = agent_id.into();
        self
    }

    /// Create a new FsBridge with no allowed paths (denies all access).
    pub fn empty() -> Self {
        Self {
            allowed_paths: HashSet::new(),
            base_dir: PathBuf::from("."),
            audit_handle: None,
            agent_id: String::new(),
        }
    }

    /// Returns current Unix time in milliseconds.
    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Fire-and-forget audit event (sync, non-blocking).
    fn audit(&self, event: AuditEvent) {
        if let Some(h) = &self.audit_handle {
            h.send_blocking(event);
        }
    }

    /// Validate that the given path is within allowed directories.
    ///
    /// Returns the canonicalized path on success, or an error message on failure.
    fn validate_path(&self, path: &str) -> Result<PathBuf, String> {
        // Check against allowed paths first (before any filesystem operations)
        if self.allowed_paths.is_empty() {
            return Err(format!(
                "Permission denied: no filesystem access allowed (path: '{}')",
                path
            ));
        }

        // Parse and resolve the path
        let path_obj = Path::new(path);
        let resolved = if path_obj.is_absolute() {
            path_obj.to_path_buf()
        } else {
            self.base_dir.join(path_obj)
        };

        // Canonicalize to resolve symlinks and normalize
        let canonical = match resolved.canonicalize() {
            Ok(c) => c,
            Err(_) => {
                // If we can't canonicalize, try to at least check for directory traversal
                // by checking if the resolved path starts with the base_dir
                let base_canonical = self
                    .base_dir
                    .canonicalize()
                    .map_err(|e| format!("Failed to resolve base directory: {}", e))?;

                // For relative paths, check they don't escape base_dir
                if !path_obj.is_absolute() {
                    let normalized = self.normalize_path(&resolved);
                    if !normalized.starts_with(&base_canonical) {
                        return Err(format!(
                            "Permission denied: path '{}' is outside allowed directories",
                            path
                        ));
                    }
                }

                return Err(format!(
                    "Failed to resolve path '{}': No such file or directory",
                    path
                ));
            }
        };

        for allowed in &self.allowed_paths {
            // Also canonicalize allowed paths for comparison
            let allowed_canonical = allowed.canonicalize().unwrap_or_else(|_| allowed.clone());
            if canonical.starts_with(&allowed_canonical) {
                return Ok(canonical);
            }
        }

        Err(format!(
            "Permission denied: path '{}' is outside allowed directories",
            canonical.display()
        ))
    }

    /// Normalize a path by resolving ".." and "." components (without requiring the path to exist).
    fn normalize_path(&self, path: &Path) -> PathBuf {
        let mut result = PathBuf::new();
        for component in path.components() {
            match component {
                std::path::Component::ParentDir => {
                    result.pop();
                }
                std::path::Component::Normal(c) => {
                    result.push(c);
                }
                std::path::Component::RootDir => {
                    result.push("/");
                }
                _ => {}
            }
        }
        result
    }

    /// Read file contents as string.
    pub fn read_file(&self, path: &str) -> Result<String, String> {
        let valid_path = self.validate_path(path)?;

        let metadata = std::fs::metadata(&valid_path)
            .map_err(|e| format!("无法获取 '{}' 的文件信息: {}", valid_path.display(), e))?;

        if metadata.len() > MAX_READ_FILE_SIZE {
            return Err(format!(
                "文件 '{}' 过大无法读取（{} 字节；上限为 {} 字节）",
                valid_path.display(),
                metadata.len(),
                MAX_READ_FILE_SIZE
            ));
        }

        let content = std::fs::read_to_string(&valid_path)
            .map_err(|e| format!("读取文件 '{}' 失败: {}", valid_path.display(), e))?;

        self.audit(AuditEvent::ScriptFsRead {
            timestamp_ms: Self::now_ms(),
            agent_id: self.agent_id.clone(),
            path: valid_path.to_string_lossy().into_owned(),
            bytes: content.len(),
        });

        Ok(content)
    }

    /// Validate that a parent directory is within allowed paths.
    fn validate_parent_dir(&self, path: &str) -> Result<PathBuf, String> {
        // Check if allowed_paths is empty
        if self.allowed_paths.is_empty() {
            return Err(format!(
                "Permission denied: no filesystem access allowed (path: '{}')",
                path
            ));
        }

        let path_obj = Path::new(path);
        let resolved = if path_obj.is_absolute() {
            path_obj.to_path_buf()
        } else {
            self.base_dir.join(path_obj)
        };

        // Get parent directory
        let parent = resolved
            .parent()
            .ok_or_else(|| format!("Cannot write to root path: '{}'", path))?;

        // Canonicalize parent
        let parent_canonical = parent.canonicalize().map_err(|e| {
            format!(
                "Failed to resolve parent directory '{}': {}",
                parent.display(),
                e
            )
        })?;

        // Check if parent is within allowed paths
        for allowed in &self.allowed_paths {
            let allowed_canonical = allowed.canonicalize().unwrap_or_else(|_| allowed.clone());
            if parent_canonical.starts_with(&allowed_canonical) {
                return Ok(resolved);
            }
        }

        Err(format!(
            "Permission denied: path '{}' is outside allowed directories",
            parent_canonical.display()
        ))
    }

    /// Write string contents to file.
    pub fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        // For write operations, we validate the parent directory exists and is allowed
        let target_path = self.validate_parent_dir(path)?;
        std::fs::write(&target_path, content)
            .map_err(|e| format!("Failed to write file '{}': {}", target_path.display(), e))?;

        self.audit(AuditEvent::ScriptFsWrite {
            timestamp_ms: Self::now_ms(),
            agent_id: self.agent_id.clone(),
            path: target_path.to_string_lossy().into_owned(),
            bytes: content.len(),
        });

        Ok(())
    }

    /// Check if path exists.
    pub fn exists(&self, path: &str) -> Result<bool, String> {
        // First check if allowed_paths is empty
        if self.allowed_paths.is_empty() {
            return Err(format!(
                "Permission denied: no filesystem access allowed (path: '{}')",
                path
            ));
        }

        // Parse and resolve the path
        let path_obj = Path::new(path);
        let resolved = if path_obj.is_absolute() {
            path_obj.to_path_buf()
        } else {
            self.base_dir.join(path_obj)
        };

        // For exists check, we need to validate the parent directory is allowed
        // but we can't canonicalize a non-existent file
        let parent = resolved.parent().unwrap_or(&resolved);
        let parent_canonical = parent
            .canonicalize()
            .map_err(|e| format!("Failed to resolve path '{}': {}", path, e))?;

        // Check if parent is within allowed paths
        let mut parent_allowed = false;
        for allowed in &self.allowed_paths {
            let allowed_canonical = allowed.canonicalize().unwrap_or_else(|_| allowed.clone());
            if parent_canonical.starts_with(&allowed_canonical) {
                parent_allowed = true;
                break;
            }
        }

        if !parent_allowed {
            return Err(format!(
                "Permission denied: path '{}' is outside allowed directories",
                parent_canonical.display()
            ));
        }

        Ok(resolved.exists())
    }

    /// List directory contents.
    /// Returns a table of entries with `name` and `is_dir` fields.
    pub fn list_dir(&self, path: &str) -> Result<Vec<DirEntry>, String> {
        let valid_path = self.validate_path(path)?;

        if !valid_path.is_dir() {
            return Err(format!("'{}' is not a directory", valid_path.display()));
        }

        let entries: Result<Vec<DirEntry>, String> = std::fs::read_dir(&valid_path)
            .map_err(|e| format!("Failed to read directory '{}': {}", valid_path.display(), e))?
            .take(MAX_LIST_DIR_ENTRIES)
            .map(|entry| {
                entry
                    .map_err(|e| format!("Failed to read directory entry: {}", e))
                    .and_then(|e| {
                        let name = e
                            .file_name()
                            .into_string()
                            .map_err(|_| "Invalid filename encoding".to_string())?;
                        let is_dir = e.file_type().map_err(|e| e.to_string())?.is_dir();
                        Ok(DirEntry { name, is_dir })
                    })
            })
            .collect();

        entries
    }

    /// Glob files matching the pattern that are within allowed directories.
    pub fn glob_files(&self, pattern: &str) -> Result<Vec<String>, String> {
        if self.allowed_paths.is_empty() {
            return Err("Permission denied: no filesystem access allowed".to_string());
        }

        let matches: Vec<String> = glob::glob(pattern)
            .map_err(|e| format!("Invalid glob pattern '{}': {}", pattern, e))?
            .filter_map(|entry| entry.ok())
            .filter(|path| {
                // Only include paths within allowed directories
                let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
                self.allowed_paths.iter().any(|base| {
                    let base_canon = base.canonicalize().unwrap_or_else(|_| base.clone());
                    canonical.starts_with(&base_canon)
                })
            })
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        self.audit(AuditEvent::ScriptFsGlob {
            timestamp_ms: Self::now_ms(),
            agent_id: self.agent_id.clone(),
            pattern: pattern.to_owned(),
            matches: matches.len(),
        });

        Ok(matches)
    }

    /// Create a directory.
    pub fn mkdir(&self, path: &str) -> Result<(), String> {
        // For mkdir operations, we validate the parent directory exists and is allowed
        let path_obj = Path::new(path);
        let resolved = if path_obj.is_absolute() {
            path_obj.to_path_buf()
        } else {
            self.base_dir.join(path_obj)
        };

        // For creating a directory, we need to validate the parent of the target directory
        // (unless it's a single-level directory in the base_dir)
        let parent = if resolved
            .parent()
            .map(|p| p.as_os_str().is_empty())
            .unwrap_or(true)
        {
            // Target is directly in base_dir
            self.base_dir.clone()
        } else {
            resolved.parent().unwrap().to_path_buf()
        };

        // Validate the parent path
        if self.allowed_paths.is_empty() {
            return Err(format!(
                "Permission denied: no filesystem access allowed (path: '{}')",
                path
            ));
        }

        let parent_canonical = parent.canonicalize().map_err(|e| {
            format!(
                "Failed to resolve parent directory '{}': {}",
                parent.display(),
                e
            )
        })?;

        let mut parent_allowed = false;
        for allowed in &self.allowed_paths {
            let allowed_canonical = allowed.canonicalize().unwrap_or_else(|_| allowed.clone());
            if parent_canonical.starts_with(&allowed_canonical) {
                parent_allowed = true;
                break;
            }
        }

        if !parent_allowed {
            return Err(format!(
                "Permission denied: path '{}' is outside allowed directories",
                parent_canonical.display()
            ));
        }

        std::fs::create_dir_all(&resolved)
            .map_err(|e| format!("Failed to create directory '{}': {}", resolved.display(), e))
    }
}

/// Directory entry for list_dir results.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

impl UserData for DirEntry {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("name", |_, this, ()| Ok(this.name.clone()));
        methods.add_method("is_dir", |_, this, ()| Ok(this.is_dir));
    }
}

impl UserData for FsBridge {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("read_file", |_, this, path: String| {
            this.read_file(&path).map_err(mlua::Error::runtime)
        });

        methods.add_method(
            "write_file",
            |_, this, (path, content): (String, String)| {
                this.write_file(&path, &content)
                    .map_err(mlua::Error::runtime)
            },
        );

        methods.add_method("exists", |_, this, path: String| {
            this.exists(&path).map_err(mlua::Error::runtime)
        });

        methods.add_method("list_dir", |lua, this, path: String| {
            let entries = this.list_dir(&path).map_err(mlua::Error::runtime)?;

            // Create a Lua table from the entries
            let table = lua.create_table()?;
            for (i, entry) in entries.into_iter().enumerate() {
                let entry_table = lua.create_table()?;
                entry_table.set("name", entry.name)?;
                entry_table.set("is_dir", entry.is_dir)?;
                table.raw_set(i + 1, entry_table)?;
            }
            Ok(table)
        });

        methods.add_method("mkdir", |_, this, path: String| {
            this.mkdir(&path).map_err(mlua::Error::runtime)
        });

        methods.add_method("glob", |lua, this, pattern: String| {
            let matches = this.glob_files(&pattern).map_err(mlua::Error::runtime)?;
            let table = lua.create_table()?;
            for (i, path) in matches.into_iter().enumerate() {
                table.raw_set(i + 1, path)?;
            }
            Ok(table)
        });
    }
}

/// Register the FsBridge as a global `fs` table in the Lua instance.
///
/// # Example in Lua:
/// ```lua
/// local content = fs:read_file("/allowed/path/file.txt")
/// fs:write_file("/allowed/path/output.txt", "Hello, World!")
/// local exists = fs:exists("/allowed/path/file.txt")
/// local entries = fs:list_dir("/allowed/path")
/// for _, entry in ipairs(entries) do
///     print(entry.name, entry.is_dir)
/// end
/// fs:mkdir("/allowed/path/new_dir")
/// ```
pub fn register_fs(lua: &Lua, bridge: FsBridge) -> LuaResult<()> {
    lua.globals().set("fs", bridge)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn create_temp_dir() -> PathBuf {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir().join(format!(
            "claw_script_test_{}_{}",
            std::process::id(),
            counter
        ));
        // Clean up any existing directory first
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        temp_dir
    }

    fn cleanup_temp_dir(path: &Path) {
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn test_fs_bridge_validate_path_within_allowed() {
        let temp_dir = create_temp_dir();
        let allowed_path = temp_dir.join("allowed");
        std::fs::create_dir(&allowed_path).unwrap();

        let bridge = FsBridge::new(vec![allowed_path.clone()], &temp_dir);

        // Create a test file
        let test_file = allowed_path.join("test.txt");
        std::fs::write(&test_file, "test content").unwrap();

        let result = bridge.validate_path("allowed/test.txt");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), test_file.canonicalize().unwrap());

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn test_fs_bridge_validate_path_outside_allowed() {
        let temp_dir = create_temp_dir();
        let allowed_path = temp_dir.join("allowed");
        std::fs::create_dir(&allowed_path).unwrap();

        let outside_dir = std::env::temp_dir().join(format!("outside_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&outside_dir);
        std::fs::create_dir_all(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("secret.txt"), "secret").unwrap();

        let bridge = FsBridge::new(vec![allowed_path], &temp_dir);

        let path_str = outside_dir.join("secret.txt").to_str().unwrap().to_string();
        let result = bridge.validate_path(&path_str);
        assert!(result.is_err());
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("Permission denied"),
            "Expected 'Permission denied' but got: {}",
            err_msg
        );

        cleanup_temp_dir(&temp_dir);
        let _ = std::fs::remove_dir_all(&outside_dir);
    }

    #[test]
    fn test_fs_bridge_empty_denies_all() {
        let bridge = FsBridge::empty();

        let result = bridge.validate_path("/any/path.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no filesystem access allowed"));
    }

    #[test]
    fn test_fs_bridge_read_file_success() {
        let temp_dir = create_temp_dir();
        let test_file = temp_dir.join("test.txt");
        std::fs::write(&test_file, "Hello, World!").unwrap();

        let bridge = FsBridge::new(vec![temp_dir.clone()], &temp_dir);

        let content = bridge.read_file("test.txt").unwrap();
        assert_eq!(content, "Hello, World!");

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn test_fs_bridge_write_file_success() {
        let temp_dir = create_temp_dir();
        let bridge = FsBridge::new(vec![temp_dir.clone()], &temp_dir);

        bridge.write_file("output.txt", "Test content").unwrap();

        let content = std::fs::read_to_string(temp_dir.join("output.txt")).unwrap();
        assert_eq!(content, "Test content");

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn test_fs_bridge_exists() {
        let temp_dir = create_temp_dir();
        let test_file = temp_dir.join("exists.txt");
        std::fs::write(&test_file, "").unwrap();

        let bridge = FsBridge::new(vec![temp_dir.clone()], &temp_dir);

        assert!(bridge.exists("exists.txt").unwrap());
        assert!(!bridge.exists("nonexistent.txt").unwrap());

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn test_fs_bridge_list_dir() {
        let temp_dir = create_temp_dir();
        std::fs::write(temp_dir.join("file1.txt"), "").unwrap();
        std::fs::create_dir(temp_dir.join("subdir")).unwrap();

        let bridge = FsBridge::new(vec![temp_dir.clone()], &temp_dir);

        let entries = bridge.list_dir(".").unwrap();
        assert_eq!(entries.len(), 2);

        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"file1.txt"));
        assert!(names.contains(&"subdir"));

        let subdir_entry = entries.iter().find(|e| e.name == "subdir").unwrap();
        assert!(subdir_entry.is_dir);

        let file_entry = entries.iter().find(|e| e.name == "file1.txt").unwrap();
        assert!(!file_entry.is_dir);

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn test_fs_bridge_mkdir() {
        let temp_dir = create_temp_dir();
        let bridge = FsBridge::new(vec![temp_dir.clone()], &temp_dir);

        bridge.mkdir("new_directory").unwrap();
        assert!(temp_dir.join("new_directory").is_dir());

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn test_fs_bridge_permission_denied_read() {
        let temp_dir = create_temp_dir();
        let allowed = temp_dir.join("allowed");
        let denied = temp_dir.join("denied");
        std::fs::create_dir(&allowed).unwrap();
        std::fs::create_dir(&denied).unwrap();
        std::fs::write(denied.join("secret.txt"), "secret").unwrap();

        let bridge = FsBridge::new(vec![allowed], &temp_dir);

        let result = bridge.read_file("../denied/secret.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Permission denied"));

        cleanup_temp_dir(&temp_dir);
    }

    #[tokio::test]
    async fn test_fs_bridge_audit_read_emits_event() {
        use claw_tools::audit::{AuditLogConfig, AuditLogWriter};

        let temp_dir = create_temp_dir();
        let test_file = temp_dir.join("hello.txt");
        std::fs::write(&test_file, "audit test content").unwrap();

        let config = AuditLogConfig::new().with_log_dir(temp_dir.join("logs"));
        let (handle, store, _task) = AuditLogWriter::start(config);

        let bridge = FsBridge::new(vec![temp_dir.clone()], &temp_dir)
            .with_audit(handle, "agent-audit-test");

        let content = bridge.read_file("hello.txt").unwrap();
        assert_eq!(content, "audit test content");

        // Give the audit store time to receive the event via mpsc
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let events = store.list(10, Some("agent-audit-test"), None);
        assert_eq!(events.len(), 1, "expected exactly one audit event");
        match &events[0] {
            claw_tools::audit::AuditEvent::ScriptFsRead { agent_id, bytes, .. } => {
                assert_eq!(agent_id, "agent-audit-test");
                assert_eq!(*bytes, "audit test content".len());
            }
            other => panic!("unexpected audit event type: {:?}", other),
        }

        cleanup_temp_dir(&temp_dir);
    }

    #[tokio::test]
    async fn test_fs_bridge_audit_write_emits_event() {
        use claw_tools::audit::{AuditLogConfig, AuditLogWriter};

        let temp_dir = create_temp_dir();

        let config = AuditLogConfig::new().with_log_dir(temp_dir.join("logs"));
        let (handle, store, _task) = AuditLogWriter::start(config);

        let bridge = FsBridge::new(vec![temp_dir.clone()], &temp_dir)
            .with_audit(handle, "agent-write-test");

        bridge.write_file("out.txt", "hello write").unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let events = store.list(10, Some("agent-write-test"), None);
        assert_eq!(events.len(), 1);
        match &events[0] {
            claw_tools::audit::AuditEvent::ScriptFsWrite { agent_id, bytes, .. } => {
                assert_eq!(agent_id, "agent-write-test");
                assert_eq!(*bytes, "hello write".len());
            }
            other => panic!("unexpected audit event type: {:?}", other),
        }

        cleanup_temp_dir(&temp_dir);
    }

    #[tokio::test]
    async fn test_fs_bridge_audit_glob_emits_event() {
        use claw_tools::audit::{AuditLogConfig, AuditLogWriter};

        let temp_dir = create_temp_dir();
        std::fs::write(temp_dir.join("a.txt"), "a").unwrap();
        std::fs::write(temp_dir.join("b.txt"), "b").unwrap();

        let config = AuditLogConfig::new().with_log_dir(temp_dir.join("logs"));
        let (handle, store, _task) = AuditLogWriter::start(config);

        let bridge = FsBridge::new(vec![temp_dir.clone()], &temp_dir)
            .with_audit(handle, "agent-glob-test");

        let pattern = format!("{}/*.txt", temp_dir.display());
        let matches = bridge.glob_files(&pattern).unwrap();
        assert_eq!(matches.len(), 2);

        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

        let events = store.list(10, Some("agent-glob-test"), None);
        assert_eq!(events.len(), 1);
        match &events[0] {
            claw_tools::audit::AuditEvent::ScriptFsGlob { agent_id, matches, .. } => {
                assert_eq!(agent_id, "agent-glob-test");
                assert_eq!(*matches, 2);
            }
            other => panic!("unexpected audit event type: {:?}", other),
        }

        cleanup_temp_dir(&temp_dir);
    }

    #[test]
    fn test_fs_bridge_no_audit_when_handle_absent() {
        // Without with_audit(), operations succeed silently — no panic.
        let temp_dir = create_temp_dir();
        let test_file = temp_dir.join("silent.txt");
        std::fs::write(&test_file, "no audit").unwrap();

        let bridge = FsBridge::new(vec![temp_dir.clone()], &temp_dir);
        let content = bridge.read_file("silent.txt").unwrap();
        assert_eq!(content, "no audit");

        cleanup_temp_dir(&temp_dir);
    }
}
