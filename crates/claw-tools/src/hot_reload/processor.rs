//! Hot-reload processor for handling file changes and triggering tool updates.
//!
//! Coordinates between file watcher and tool registry to enable tool reloading.
//!
//! # Version Management
//!
//! This module now uses [`VersionedModule`] for atomic hot-swapping of compiled tools.
//! See the `versioned` module for low-level version management primitives.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::error::LoadError;
use crate::hot_reload::validation::ToolWatcher;
use crate::hot_reload::versioned::VersionedModule;
use crate::hot_reload::watcher::WatchEvent;
use crate::registry::ToolRegistry;
use crate::traits::{ScriptToolCompiler, Tool};
use crate::types::HotLoadingConfig;

/// Result of processing a file change.
#[derive(Debug, Clone)]
pub enum ProcessResult {
    /// Tool was successfully loaded/reloaded.
    Loaded { tool_name: String, path: PathBuf },
    /// Tool was removed.
    Removed { tool_name: String },
    /// No action taken (e.g., file not a tool).
    Skipped { path: PathBuf, reason: String },
    /// Compilation or loading failed.
    Failed { path: PathBuf, error: String },
}

/// A previous tool version being kept alive for the TTL window so that
/// in-flight `execute()` calls can finish before the Arc is dropped.
struct RetainedVersion {
    /// The old tool Arc — dropped when this struct is dropped.
    #[allow(dead_code)]
    tool: Arc<dyn Tool>,
    /// When this version was retired (replaced by a hot-reload).
    retired_at: Instant,
}

/// Processor that handles file watch events and triggers hot-reloads.
pub struct HotReloadProcessor {
    registry: Arc<ToolRegistry>,
    config: HotLoadingConfig,
    tool_watcher: ToolWatcher,
    /// Optional script compiler injected at construction time.
    compiler: Option<Arc<dyn ScriptToolCompiler>>,
    /// Previous tool versions kept alive for `keep_previous_secs` so that
    /// any in-flight `execute()` call can complete against the old Arc.
    retained_versions: Arc<Mutex<Vec<RetainedVersion>>>,
}

impl HotReloadProcessor {
    /// Create a new hot-reload processor.
    pub fn new(registry: Arc<ToolRegistry>, config: HotLoadingConfig) -> Self {
        let tool_watcher = ToolWatcher::new(config.clone(), Arc::clone(&registry));
        Self {
            registry,
            config,
            tool_watcher,
            compiler: None,
            retained_versions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Create a new hot-reload processor with a custom tool watcher.
    ///
    /// This is useful for testing or when you need custom validation behavior.
    pub fn with_watcher(registry: Arc<ToolRegistry>, tool_watcher: ToolWatcher) -> Self {
        let config = tool_watcher.config().clone();
        Self {
            registry,
            config,
            tool_watcher,
            compiler: None,
            retained_versions: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Attach a [`ScriptToolCompiler`] so that watched script files are actually
    /// compiled and registered as live tools.
    ///
    /// Without a compiler every call to `compile_tool` returns an error.
    /// Inject a compiler to enable real script-tool hot-loading:
    ///
    /// ```rust,ignore
    /// use claw_script::LuaToolCompiler;
    /// let processor = HotReloadProcessor::new(registry, config)
    ///     .with_compiler(LuaToolCompiler::arc());
    /// ```
    pub fn with_compiler(mut self, compiler: Arc<dyn ScriptToolCompiler>) -> Self {
        self.compiler = Some(compiler);
        self
    }

    /// Run the processor, handling events from the given receiver.
    ///
    /// This method loops until the receiver is closed.
    pub async fn run(&self, mut event_rx: mpsc::Receiver<WatchEvent>) {
        while let Some(event) = event_rx.recv().await {
            let result = match event {
                WatchEvent::FileChanged(path) | WatchEvent::FileCreated(path) => {
                    self.handle_file_change(&path).await
                }
                WatchEvent::FileRemoved(path) => self.handle_file_removed(&path).await,
                WatchEvent::Debounced(paths) => {
                    // Process all debounced paths
                    let mut results = Vec::new();
                    for path in paths {
                        results.push(self.handle_file_change(&path).await);
                    }
                    // Return first result or a skipped result
                    results
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| ProcessResult::Skipped {
                            path: PathBuf::new(),
                            reason: "empty debounced batch".to_string(),
                        })
                }
            };

            // Log the result (in production, this might emit events to a broader system)
            match &result {
                ProcessResult::Loaded { tool_name, path } => {
                    tracing::info!("Hot-reloaded tool '{}' from {:?}", tool_name, path);
                }
                ProcessResult::Removed { tool_name } => {
                    tracing::info!("Removed tool '{}'", tool_name);
                }
                ProcessResult::Skipped { reason, .. } => {
                    tracing::debug!("Skipped file: {}", reason);
                }
                ProcessResult::Failed { path, error } => {
                    tracing::error!("Failed to load {:?}: {}", path, error);
                }
            }
        }
    }

    /// Handle a file change or creation event.
    ///
    /// 1. Validate file through 4-step validation pipeline
    ///    - Syntax check
    ///    - Permission audit
    ///    - Schema validation
    ///    - Sandbox compilation
    /// 2. Compile with timeout
    /// 3. Update in registry
    async fn handle_file_change(&self, path: &Path) -> ProcessResult {
        // Check extension
        if !self.is_valid_extension(path) {
            return ProcessResult::Skipped {
                path: path.to_path_buf(),
                reason: "file extension not in watch list".to_string(),
            };
        }

        // Check if file exists and is readable
        if !path.exists() {
            return ProcessResult::Skipped {
                path: path.to_path_buf(),
                reason: "file does not exist".to_string(),
            };
        }

        // Step 1-4: Run full validation pipeline
        if let Err(validation_err) = self.tool_watcher.validate(path).await {
            tracing::error!("Validation failed for {:?}: {}", path, validation_err);
            return ProcessResult::Failed {
                path: path.to_path_buf(),
                error: format!("validation failed: {}", validation_err),
            };
        }

        // Read file content
        let content = match tokio::fs::read_to_string(path).await {
            Ok(c) => c,
            Err(e) => {
                return ProcessResult::Failed {
                    path: path.to_path_buf(),
                    error: format!("read error: {}", e),
                }
            }
        };

        // Compile the tool with timeout
        let compile_timeout = Duration::from_secs(self.config.compile_timeout_secs);
        let compiled = match timeout(compile_timeout, self.compile_tool(&content, path)).await {
            Ok(Ok(tool)) => tool,
            Ok(Err(e)) => {
                return ProcessResult::Failed {
                    path: path.to_path_buf(),
                    error: format!("compile error: {}", e),
                }
            }
            Err(_) => {
                return ProcessResult::Failed {
                    path: path.to_path_buf(),
                    error: "compilation timed out".to_string(),
                }
            }
        };

        // Get tool name
        let tool_name = compiled.name().to_string();

        // Retire the previous version: keep it alive for `keep_previous_secs`
        // so that any in-flight `execute()` call against the old Arc can finish.
        self.retire_old_version(&tool_name);

        // Update the tool in the registry
        match self.registry.update(&tool_name, compiled) {
            Ok(()) => {}
            Err(e) => {
                return ProcessResult::Failed {
                    path: path.to_path_buf(),
                    error: format!("registry error: {}", e),
                }
            }
        }

        ProcessResult::Loaded {
            tool_name,
            path: path.to_path_buf(),
        }
    }

    /// Handle a file removal event.
    ///
    /// Unregisters the tool associated with this file path.
    async fn handle_file_removed(&self, path: &Path) -> ProcessResult {
        // Check if this path corresponds to a known tool
        let path_str = path.to_string_lossy();

        // Find the tool by source path
        for name in self.registry.tool_names() {
            if let Some(meta) = self.registry.tool_meta(&name) {
                if let crate::types::ToolSource::Script {
                    path: ref source_path,
                    ..
                } = meta.source
                {
                    if source_path.to_string_lossy() == path_str {
                        match self.registry.unregister(&name) {
                            Ok(()) => {
                                return ProcessResult::Removed { tool_name: name };
                            }
                            Err(e) => {
                                return ProcessResult::Failed {
                                    path: path.to_path_buf(),
                                    error: format!("unregister error: {}", e),
                                }
                            }
                        }
                    }
                }
            }
        }

        ProcessResult::Skipped {
            path: path.to_path_buf(),
            reason: "no tool registered for this path".to_string(),
        }
    }

    /// Check if a file has a valid extension.
    fn is_valid_extension(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| self.config.is_watched_extension(e))
            .unwrap_or(false)
    }

    /// Retire the currently-registered version of a tool before a hot-swap.
    ///
    /// The old [`Arc<dyn Tool>`] is moved into [`Self::retained_versions`] with the
    /// current timestamp.  It stays alive (preventing premature drop) until the
    /// `keep_previous_secs` TTL expires, giving any in-flight `execute()` call time
    /// to complete against the old implementation.
    ///
    /// Expired entries are pruned on every call so the list stays bounded.
    fn retire_old_version(&self, tool_name: &str) {
        let Some(old) = self.registry.get(tool_name) else {
            return; // First load — nothing to retire.
        };

        let ttl = Duration::from_secs(self.config.keep_previous_secs);
        let now = Instant::now();

        if let Ok(mut retained) = self.retained_versions.lock() {
            // Prune entries whose TTL has expired.
            retained.retain(|v| now.duration_since(v.retired_at) < ttl);
            // Keep the old version alive for the TTL window.
            retained.push(RetainedVersion {
                tool: old,
                retired_at: now,
            });
        }
    }

    /// Compile a tool from source content.
    ///
    /// Delegates to the injected [`ScriptToolCompiler`] when one has been set via
    /// [`Self::with_compiler`].  Returns `LoadError::CompileError` if no compiler
    /// is attached or if the compiler does not support the file's extension.
    async fn compile_tool(&self, content: &str, path: &Path) -> Result<Arc<dyn Tool>, LoadError> {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        match &self.compiler {
            Some(compiler) if compiler.supports_extension(ext) => {
                compiler.compile(path, content).await
            }
            Some(_) => {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<unknown>");
                Err(LoadError::CompileError(format!(
                    "no compiler supports extension '{}' for tool '{}'",
                    ext, name
                )))
            }
            None => {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<unknown>");
                Err(LoadError::CompileError(format!(
                    "no ScriptToolCompiler attached — call with_compiler() to enable script-tool loading (tool: '{}')",
                    name
                )))
            }
        }
    }

    /// Load a tool directly from a file path.
    ///
    /// This is a convenience method for manual loading (not via file watcher).
    pub async fn load_from_path(&self, path: &Path) -> Result<(), LoadError> {
        let content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| LoadError::Io(e.to_string()))?;

        let compile_timeout = Duration::from_secs(self.config.compile_timeout_secs);
        let compiled = timeout(compile_timeout, self.compile_tool(&content, path))
            .await
            .map_err(|_| LoadError::CompileError("compilation timed out".to_string()))??;

        let tool_name = compiled.name().to_string();

        self.registry
            .update(&tool_name, compiled)
            .map_err(|e| LoadError::CompileError(e.to_string()))
    }
}

/// Builder for configuring hot-reload.
pub struct HotReloadBuilder {
    config: HotLoadingConfig,
}

impl HotReloadBuilder {
    /// Create a new builder with default configuration.
    pub fn new() -> Self {
        Self {
            config: HotLoadingConfig::default(),
        }
    }

    /// Add a watch directory.
    pub fn watch_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.watch_dirs.push(path.into());
        self
    }

    /// Set file extensions to watch.
    pub fn extensions(mut self, exts: Vec<String>) -> Self {
        self.config.extensions = exts;
        self
    }

    /// Set debounce delay in milliseconds.
    pub fn debounce_ms(mut self, ms: u64) -> Self {
        self.config.debounce_ms = ms;
        self
    }

    /// Set compile timeout in seconds.
    pub fn compile_timeout_secs(mut self, secs: u64) -> Self {
        self.config.compile_timeout_secs = secs;
        self
    }

    /// Set auto-enable for newly loaded tools.
    pub fn auto_enable(mut self, enable: bool) -> Self {
        self.config.auto_enable = enable;
        self
    }

    /// Build the configuration.
    pub fn build(self) -> Result<HotLoadingConfig, String> {
        self.config.validate()?;
        Ok(self.config)
    }
}

impl Default for HotReloadBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// A versioned set of compiled tools for atomic hot-swapping.
///
/// Wraps a collection of tools with version management, enabling:
/// - Zero-downtime updates
/// - Rollback to previous versions
/// - Lock-free concurrent access
///
/// # Example
///
/// ```rust,no_run
/// use std::sync::Arc;
/// use claw_tools::hot_reload::VersionedToolSet;
///
/// # fn example() {
/// let tools = VersionedToolSet::new();
///
/// // Load initial tools
/// let mut initial_tools = std::collections::HashMap::new();
/// // ... populate tools ...
/// let v1 = tools.swap(initial_tools);
///
/// // Later, atomically swap to new version
/// let mut new_tools = std::collections::HashMap::new();
/// // ... populate new tools ...
/// let v2 = tools.swap(new_tools);
/// # }
/// ```
pub struct VersionedToolSet {
    /// Versioned storage for the tool collection.
    inner: VersionedModule<HashMap<String, Arc<dyn Tool>>>,
}

impl std::fmt::Debug for VersionedToolSet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VersionedToolSet")
            .field("version", &self.version())
            .field("len", &self.len())
            .finish()
    }
}

impl VersionedToolSet {
    /// Create a new empty versioned tool set.
    pub fn new() -> Self {
        Self {
            inner: VersionedModule::new(Arc::new(HashMap::new())),
        }
    }

    /// Create a new versioned tool set with initial tools and custom history size.
    ///
    /// # Arguments
    ///
    /// * `initial` - Initial set of tools
    /// * `max_history` - Maximum number of versions to retain
    pub fn with_capacity(initial: HashMap<String, Arc<dyn Tool>>, max_history: usize) -> Self {
        Self {
            inner: VersionedModule::with_capacity(Arc::new(initial), max_history),
        }
    }

    /// Atomically swap to a new set of tools.
    ///
    /// Returns the new version number. Existing readers continue to see
    /// the previous version until they refresh their reference.
    pub fn swap(&self, tools: HashMap<String, Arc<dyn Tool>>) -> u64 {
        self.inner.swap(Arc::new(tools))
    }

    /// Load the current tool set.
    ///
    /// This is lock-free and returns a snapshot that remains valid
    /// even if a hot-swap occurs.
    pub fn load(&self) -> Arc<HashMap<String, Arc<dyn Tool>>> {
        self.inner.load()
    }

    /// Get a specific tool by name.
    ///
    /// Returns `Some(tool)` if found, `None` otherwise.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.load().get(name).cloned()
    }

    /// Check if a tool exists.
    pub fn contains(&self, name: &str) -> bool {
        self.load().contains_key(name)
    }

    /// Get the number of tools in the current version.
    pub fn len(&self) -> usize {
        self.load().len()
    }

    /// Check if the tool set is empty.
    pub fn is_empty(&self) -> bool {
        self.load().is_empty()
    }

    /// Get the current version number.
    pub fn version(&self) -> u64 {
        self.inner.current_version()
    }

    /// Rollback to a specific version.
    ///
    /// Returns `true` if successful, `false` if version not found.
    pub fn rollback_to(&self, version: u64) -> bool {
        self.inner.rollback(version)
    }

    /// Rollback to the previous version.
    ///
    /// Returns `true` if successful, `false` if no previous version.
    pub fn rollback(&self) -> bool {
        self.inner.rollback_previous()
    }

    /// Get the version history.
    pub fn versions(&self) -> Vec<(u64, usize)> {
        self.inner
            .versions()
            .into_iter()
            .map(|v| (v.version, v.module.len()))
            .collect()
    }
}

impl Default for VersionedToolSet {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> HotLoadingConfig {
        HotLoadingConfig {
            watch_dirs: vec![PathBuf::from("/tmp/tools")],
            extensions: vec!["lua".to_string()],
            debounce_ms: 50,
            default_timeout_secs: 30,
            compile_timeout_secs: 10,
            keep_previous_secs: 300,
            auto_enable: true,
        }
    }

    #[test]
    fn test_process_result_variants() {
        let loaded = ProcessResult::Loaded {
            tool_name: "test".to_string(),
            path: PathBuf::from("/test.lua"),
        };
        assert!(matches!(loaded, ProcessResult::Loaded { .. }));

        let removed = ProcessResult::Removed {
            tool_name: "test".to_string(),
        };
        assert!(matches!(removed, ProcessResult::Removed { .. }));

        let skipped = ProcessResult::Skipped {
            path: PathBuf::from("/test.txt"),
            reason: "wrong extension".to_string(),
        };
        assert!(matches!(skipped, ProcessResult::Skipped { .. }));

        let failed = ProcessResult::Failed {
            path: PathBuf::from("/test.lua"),
            error: "syntax error".to_string(),
        };
        assert!(matches!(failed, ProcessResult::Failed { .. }));
    }

    #[test]
    fn test_hot_reload_builder() {
        let config = HotReloadBuilder::new()
            .watch_dir("/tools")
            .watch_dir("/more_tools")
            .debounce_ms(100)
            .compile_timeout_secs(15)
            .auto_enable(false)
            .extensions(vec!["lua".to_string(), "js".to_string()])
            .build()
            .unwrap();

        assert_eq!(config.watch_dirs.len(), 3); // 2 new + 1 default
        assert_eq!(config.debounce_ms, 100);
        assert_eq!(config.compile_timeout_secs, 15);
        assert!(!config.auto_enable);
        assert_eq!(config.extensions, vec!["lua", "js"]);
    }

    #[test]
    fn test_hot_reload_builder_validation() {
        let result = HotReloadBuilder::new()
            .debounce_ms(0) // Invalid
            .build();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_processor_is_valid_extension() {
        let registry = Arc::new(ToolRegistry::new());
        let config = test_config();
        let processor = HotReloadProcessor::new(registry, config);

        assert!(processor.is_valid_extension(Path::new("/test.lua")));
        assert!(!processor.is_valid_extension(Path::new("/test.py")));
        assert!(!processor.is_valid_extension(Path::new("/test")));
    }

    #[tokio::test]
    async fn test_handle_file_change_skips_nonexistent() {
        let registry = Arc::new(ToolRegistry::new());
        let config = test_config();
        let processor = HotReloadProcessor::new(registry, config);

        let result = processor
            .handle_file_change(Path::new("/nonexistent/file.lua"))
            .await;
        assert!(matches!(result, ProcessResult::Skipped { .. }));
    }

    #[tokio::test]
    async fn test_handle_file_change_skips_wrong_extension() {
        let registry = Arc::new(ToolRegistry::new());
        let config = test_config();
        let processor = HotReloadProcessor::new(registry, config);

        // Create a temp file with wrong extension
        let temp_file = std::env::temp_dir().join("test.txt");
        tokio::fs::write(&temp_file, "test content").await.unwrap();

        let result = processor.handle_file_change(&temp_file).await;
        assert!(
            matches!(result, ProcessResult::Skipped { reason, .. } if reason.contains("extension"))
        );

        let _ = tokio::fs::remove_file(&temp_file).await;
    }

    #[tokio::test]
    async fn test_handle_file_removed_skips_unknown() {
        let registry = Arc::new(ToolRegistry::new());
        let config = test_config();
        let processor = HotReloadProcessor::new(registry, config);

        let result = processor
            .handle_file_removed(Path::new("/unknown/path.lua"))
            .await;
        assert!(
            matches!(result, ProcessResult::Skipped { reason, .. } if reason.contains("no tool"))
        );
    }

    #[test]
    fn test_retire_old_version_noop_when_empty_registry() {
        let registry = Arc::new(ToolRegistry::new());
        let config = test_config();
        let processor = HotReloadProcessor::new(registry, config);

        // retire on an empty registry must not panic and leave retained list empty
        processor.retire_old_version("nonexistent");
        assert_eq!(
            processor.retained_versions.lock().unwrap().len(),
            0,
            "no version to retire"
        );
    }

    #[test]
    fn test_retire_old_version_keeps_arc_alive() {
        use crate::traits::Tool;
        use crate::types::{PermissionSet, ToolContext, ToolResult, ToolSchema};
        use async_trait::async_trait;

        struct DummyTool;

        #[async_trait]
        impl Tool for DummyTool {
            fn name(&self) -> &str { "dummy" }
            fn description(&self) -> &str { "dummy" }
            fn schema(&self) -> &ToolSchema {
                static S: std::sync::OnceLock<ToolSchema> = std::sync::OnceLock::new();
                S.get_or_init(|| ToolSchema::new("dummy", "dummy", serde_json::json!({})))
            }
            fn permissions(&self) -> &PermissionSet {
                static P: std::sync::OnceLock<PermissionSet> = std::sync::OnceLock::new();
                P.get_or_init(PermissionSet::minimal)
            }
            async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
                ToolResult::ok(serde_json::json!({}), 0)
            }
        }

        let registry = Arc::new(ToolRegistry::new());
        registry.register(Box::new(DummyTool)).unwrap();

        let config = test_config();
        let processor = HotReloadProcessor::new(Arc::clone(&registry), config);

        // Capture a weak reference to the inner tool Arc
        let old_tool = registry.get("dummy").expect("must exist");
        let weak = Arc::downgrade(&old_tool);
        drop(old_tool); // drop local reference — registry still holds one

        processor.retire_old_version("dummy");

        // retained_versions now holds the old Arc, so weak must still be upgradable
        assert!(
            weak.upgrade().is_some(),
            "old Arc must stay alive while in retained_versions"
        );

        // Verify one entry in retained list
        assert_eq!(processor.retained_versions.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_retire_old_version_prunes_expired_entries() {
        use crate::traits::Tool;
        use crate::types::{PermissionSet, ToolContext, ToolResult, ToolSchema};
        use async_trait::async_trait;

        struct DummyTool2;

        #[async_trait]
        impl Tool for DummyTool2 {
            fn name(&self) -> &str { "dummy2" }
            fn description(&self) -> &str { "dummy2" }
            fn schema(&self) -> &ToolSchema {
                static S: std::sync::OnceLock<ToolSchema> = std::sync::OnceLock::new();
                S.get_or_init(|| ToolSchema::new("dummy2", "dummy2", serde_json::json!({})))
            }
            fn permissions(&self) -> &PermissionSet {
                static P: std::sync::OnceLock<PermissionSet> = std::sync::OnceLock::new();
                P.get_or_init(PermissionSet::minimal)
            }
            async fn execute(&self, _args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
                ToolResult::ok(serde_json::json!({}), 0)
            }
        }

        let registry = Arc::new(ToolRegistry::new());
        registry.register(Box::new(DummyTool2)).unwrap();

        // Config with keep_previous_secs = 0 means every retained entry is immediately expired
        let config = HotLoadingConfig {
            keep_previous_secs: 0,
            ..test_config()
        };
        let processor = HotReloadProcessor::new(Arc::clone(&registry), config);

        // Seed one entry that is already past TTL
        {
            let old = registry.get("dummy2").unwrap();
            processor.retained_versions.lock().unwrap().push(RetainedVersion {
                tool: old,
                retired_at: Instant::now() - Duration::from_secs(1),
            });
        }
        assert_eq!(processor.retained_versions.lock().unwrap().len(), 1);

        // retire_old_version prunes expired before adding new
        processor.retire_old_version("dummy2");

        // The expired entry should be gone; the new one added (len == 1, not 2)
        assert_eq!(
            processor.retained_versions.lock().unwrap().len(),
            1,
            "expired entry pruned, new entry added"
        );
    }
}
