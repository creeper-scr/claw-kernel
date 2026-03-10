//! Hot-reload manager for script modules.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::error::ScriptError;
use crate::hot_reload::config::HotReloadConfig;
use crate::hot_reload::events::{ScriptEvent, ScriptEventBus};
use crate::hot_reload::module::{ScriptEntry, ScriptModule, ScriptRegistry};
use crate::hot_reload::watcher::{ScriptWatcher, WatchEvent};
use crate::traits::ScriptEngine;
use crate::types::{EngineType, ScriptContext};

/// Manager for script hot-reloading.
///
/// Coordinates file watching, compilation, validation, and event emission
/// for script hot-reload at Layer 3.
pub struct HotReloadManager {
    config: HotReloadConfig,
    registry: Arc<ScriptRegistry>,
    engine: Arc<dyn ScriptEngine>,
    event_bus: Arc<ScriptEventBus>,
    watcher: Option<ScriptWatcher>,
}

impl std::fmt::Debug for HotReloadManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotReloadManager")
            .field("config", &self.config)
            .field("registry", &self.registry)
            .field("engine_type", &self.engine.engine_type())
            .field("has_watcher", &self.watcher.is_some())
            .finish()
    }
}

impl HotReloadManager {
    /// Create a new hot-reload manager.
    pub fn new(
        config: HotReloadConfig,
        engine: Arc<dyn ScriptEngine>,
    ) -> Result<Self, ScriptError> {
        config.validate().map_err(|e| {
            ScriptError::Runtime(format!("Invalid hot-reload config: {}", e))
        })?;

        let registry = Arc::new(ScriptRegistry::new(config.max_history_size));
        let event_bus = Arc::new(ScriptEventBus::new(128));

        Ok(Self {
            config,
            registry,
            engine,
            event_bus,
            watcher: None,
        })
    }

    /// Subscribe to script events.
    pub fn subscribe(&self) -> broadcast::Receiver<ScriptEvent> {
        self.event_bus.subscribe()
    }

    /// Get a reference to the event bus.
    pub fn event_bus(&self) -> Arc<ScriptEventBus> {
        self.event_bus.clone()
    }

    /// Get a reference to the registry.
    pub fn registry(&self) -> Arc<ScriptRegistry> {
        self.registry.clone()
    }

    /// Get the engine type.
    pub fn engine_type(&self) -> &str {
        self.engine.engine_type()
    }

    /// Watch an additional directory.
    pub async fn watch_directory(&mut self, path: impl Into<PathBuf>) -> Result<(), ScriptError> {
        let path = path.into();
        
        if !path.exists() {
            tokio::fs::create_dir_all(&path).await.map_err(|e| {
                ScriptError::Runtime(format!("Failed to create directory {:?}: {}", path, e))
            })?;
        }

        self.config.watch_dirs.push(path);
        
        // Recreate watcher if already running
        if self.watcher.is_some() {
            self.stop_watching().await?;
            self.start_watching().await?;
        }

        Ok(())
    }

    /// Start watching for file changes.
    async fn start_watching(&mut self) -> Result<(), ScriptError> {
        if self.watcher.is_some() {
            return Ok(());
        }

        let watcher = ScriptWatcher::new(
            self.config.clone(),
            Some(self.event_bus.clone()),
        )?;

        self.watcher = Some(watcher);
        info!("Started watching directories: {:?}", self.config.watch_dirs);

        Ok(())
    }

    /// Stop watching for file changes.
    async fn stop_watching(&mut self) -> Result<(), ScriptError> {
        self.watcher = None;
        let _ = self.event_bus.emit(ScriptEvent::Stopped);
        info!("Stopped watching directories");
        Ok(())
    }

    /// Start the hot-reload event loop.
    ///
    /// This runs until the watcher is stopped or encounters an error.
    pub async fn start(&mut self) -> Result<(), ScriptError> {
        self.start_watching().await?;

        let mut watcher = self
            .watcher
            .take()
            .ok_or_else(|| ScriptError::Runtime("Watcher not initialized".to_string()))?;

        info!("Hot-reload manager started");

        while let Some(event) = watcher.recv().await {
            self.handle_watch_event(event).await;
        }

        warn!("Hot-reload event loop ended");
        Ok(())
    }

    /// Run the hot-reload manager with a cancellation token.
    pub async fn run_with_cancel(
        &mut self,
        mut cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<(), ScriptError> {
        self.start_watching().await?;

        let mut watcher = self
            .watcher
            .take()
            .ok_or_else(|| ScriptError::Runtime("Watcher not initialized".to_string()))?;

        info!("Hot-reload manager started with cancellation support");

        loop {
            tokio::select! {
                Some(event) = watcher.recv() => {
                    self.handle_watch_event(event).await;
                }
                Ok(()) = cancel.changed() => {
                    if *cancel.borrow() {
                        info!("Cancellation requested, stopping hot-reload manager");
                        break;
                    }
                }
            }
        }

        self.stop_watching().await?;
        Ok(())
    }

    /// Handle a watch event.
    async fn handle_watch_event(&self, event: WatchEvent) {
        match event {
            WatchEvent::FileCreated(path) | WatchEvent::FileChanged(path) => {
                debug!("File changed: {:?}", path);
                if let Err(e) = self.handle_file_change(&path).await {
                    error!("Failed to handle file change {:?}: {}", path, e);
                }
            }
            WatchEvent::FileRemoved(path) => {
                debug!("File removed: {:?}", path);
                self.handle_file_removed(&path).await;
            }
            WatchEvent::Debounced(paths) => {
                debug!("Debounced batch of {} files", paths.len());
                let _ = self.event_bus.emit(ScriptEvent::Debounced {
                    count: paths.len(),
                    paths: paths.clone(),
                });

                for path in paths {
                    if path.exists() {
                        if let Err(e) = self.handle_file_change(&path).await {
                            error!("Failed to handle file change {:?}: {}", path, e);
                        }
                    } else {
                        self.handle_file_removed(&path).await;
                    }
                }
            }
        }
    }

    /// Handle a file change or creation.
    async fn handle_file_change(&self, path: &PathBuf) -> Result<(), ScriptError> {
        // Check if file exists
        if !path.exists() {
            debug!("File does not exist (may have been quickly deleted): {:?}", path);
            return Ok(());
        }

        // Check extension
        if !self.config.is_watched_extension(path) {
            debug!("Ignoring file with unwatched extension: {:?}", path);
            return Ok(());
        }

        // Read file content
        let content = tokio::fs::read_to_string(path).await.map_err(|e| {
            ScriptError::Runtime(format!("Failed to read file {:?}: {}", path, e))
        })?;

        // Determine engine type
        let engine_type = self
            .config
            .engine_type_from_extension(path)
            .ok_or_else(|| {
                ScriptError::Runtime(format!("Could not determine engine type for {:?}", path))
            })?;

        // Check engine filter
        if let Some(filter) = self.config.engine_filter {
            if engine_type != filter {
                debug!("Skipping file due to engine filter: {:?}", path);
                return Ok(());
            }
        }

        // Derive script name from file stem
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| ScriptError::Runtime(format!("Invalid file name: {:?}", path)))?
            .to_string();

        // Check if this is a reload
        let is_reload = self.registry.contains(&name);

        // Validate before reload if configured
        if self.config.validate_before_reload {
            let script = match engine_type {
                EngineType::Lua => ScriptEntry::new(&name, engine_type, &content, path.clone()),
                #[cfg(feature = "engine-v8")]
                EngineType::JavaScript => ScriptEntry::new(&name, engine_type, &content, path.clone()),
                #[cfg(feature = "engine-v8")]
                EngineType::TypeScript => ScriptEntry::new(&name, engine_type, &content, path.clone()),
            };

            let script_obj = script.to_script();
            if let Err(e) = self.engine.validate(&script_obj) {
                let _ = self.event_bus.emit(ScriptEvent::Failed {
                    path: path.clone(),
                    error: e.clone(),
                    was_reload: is_reload,
                });
                return Err(e);
            }
        }

        // Create or update the script entry
        let entry = ScriptEntry::new(name.clone(), engine_type, content, path.clone());

        if is_reload {
            // Update existing
            let previous_version = self
                .registry
                .get(&name)
                .map(|e| e.version)
                .unwrap_or(0);
            
            let module = self.registry.get_or_create(&name).unwrap();
            let new_version = module.swap(entry.clone());

            let _ = self.event_bus.emit(ScriptEvent::Reloaded {
                entry: entry.clone(),
                path: path.clone(),
                previous_version,
                new_version,
            });

            info!(
                "Reloaded script '{}' from {:?} (v{} -> v{})",
                name, path, previous_version, new_version
            );
        } else {
            // Register new
            self.registry.register(entry.clone());

            let _ = self.event_bus.emit(ScriptEvent::Loaded {
                entry: entry.clone(),
                path: path.clone(),
            });

            info!("Loaded new script '{}' from {:?}", name, path);
        }

        // Emit cache update event
        let _ = self.event_bus.emit(ScriptEvent::CacheUpdated {
            name,
            engine: engine_type,
            content_hash: entry.content_hash(),
        });

        Ok(())
    }

    /// Handle a file removal.
    async fn handle_file_removed(&self, path: &PathBuf) {
        // Find the script by path
        for entry in self.registry.entries() {
            if entry.path == *path {
                let name = entry.name.clone();
                let version = entry.version;

                if self.registry.unregister(&name) {
                    let _ = self.event_bus.emit(ScriptEvent::Unloaded {
                        name: name.clone(),
                        path: path.clone(),
                        version,
                    });

                    info!("Unloaded script '{}' (file removed)", name);
                }
                break;
            }
        }
    }

    /// Load a script manually from a file path.
    pub async fn load_file(&self, path: impl Into<PathBuf>) -> Result<Arc<ScriptModule>, ScriptError> {
        let path = path.into();
        
        if !path.exists() {
            return Err(ScriptError::Runtime(format!("File not found: {:?}", path)));
        }

        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            ScriptError::Runtime(format!("Failed to read file: {}", e))
        })?;

        let engine_type = self
            .config
            .engine_type_from_extension(&path)
            .ok_or_else(|| ScriptError::Runtime("Unknown file extension".to_string()))?;

        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| ScriptError::Runtime("Invalid file name".to_string()))?;

        // Validate
        let entry = ScriptEntry::new(name, engine_type, &content, path.clone());
        let script = entry.to_script();
        self.engine.validate(&script)?;

        // Register
        let module = self.registry.register(entry);
        
        Ok(module)
    }

    /// Get a script by name.
    pub fn get_script(&self, name: &str) -> Option<Arc<ScriptEntry>> {
        self.registry.get(name)
    }

    /// Execute a script by name.
    pub async fn execute(
        &self,
        name: &str,
        ctx: &ScriptContext,
    ) -> Result<serde_json::Value, ScriptError> {
        let entry = self
            .registry
            .get(name)
            .ok_or_else(|| ScriptError::Runtime(format!("Script not found: {}", name)))?;

        let script = entry.to_script();
        self.engine.execute(&script, ctx).await
    }

    /// Rollback a script to its previous version.
    pub fn rollback(&self, name: &str) -> bool {
        self.registry.rollback(name)
    }

    /// Get all loaded script names.
    pub fn script_names(&self) -> Vec<String> {
        self.registry.names()
    }

    /// Check if a script is loaded.
    pub fn has_script(&self, name: &str) -> bool {
        self.registry.contains(name)
    }

    /// Unload a script by name.
    pub fn unload(&self, name: &str) -> bool {
        self.registry.unregister(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Script;

    // Mock engine for testing
    struct MockEngine;

    #[async_trait::async_trait]
    impl ScriptEngine for MockEngine {
        fn engine_type(&self) -> &str {
            "mock"
        }

        async fn execute(
            &self,
            _script: &Script,
            _ctx: &ScriptContext,
        ) -> Result<serde_json::Value, ScriptError> {
            Ok(serde_json::json!(42))
        }

        fn validate(&self, _script: &Script) -> Result<(), ScriptError> {
            Ok(())
        }
    }

    fn test_config() -> HotReloadConfig {
        HotReloadConfig::new()
            .watch_dir(std::env::temp_dir().join("claw_test_manager"))
            .extension("lua")
    }

    #[test]
    fn test_manager_creation() {
        let config = test_config();
        let engine: Arc<dyn ScriptEngine> = Arc::new(MockEngine);
        
        let manager = HotReloadManager::new(config, engine);
        assert!(manager.is_ok());
    }

    #[test]
    fn test_event_subscription() {
        let config = test_config();
        let engine: Arc<dyn ScriptEngine> = Arc::new(MockEngine);
        
        let manager = HotReloadManager::new(config, engine).unwrap();
        let _rx = manager.subscribe();
        
        assert_eq!(manager.event_bus.subscriber_count(), 1);
    }

    #[tokio::test]
    async fn test_manual_load() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let script_path = temp_dir.path().join("test.lua");
        tokio::fs::write(&script_path, "return 42").await.unwrap();

        let config = HotReloadConfig::new().watch_dir(temp_dir.path());
        let engine: Arc<dyn ScriptEngine> = Arc::new(MockEngine);
        
        let manager = HotReloadManager::new(config, engine).unwrap();
        let module = manager.load_file(&script_path).await.unwrap();
        
        assert_eq!(module.current().name, "test");
        assert_eq!(module.current().source, "return 42");
    }

    #[tokio::test]
    async fn test_execute_script() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let script_path = temp_dir.path().join("test.lua");
        tokio::fs::write(&script_path, "return 42").await.unwrap();

        let config = HotReloadConfig::new().watch_dir(temp_dir.path());
        let engine: Arc<dyn ScriptEngine> = Arc::new(MockEngine);
        
        let manager = HotReloadManager::new(config, engine).unwrap();
        manager.load_file(&script_path).await.unwrap();

        let ctx = ScriptContext::new("test-agent");
        let result = manager.execute("test", &ctx).await.unwrap();
        
        assert_eq!(result, serde_json::json!(42));
    }

    #[tokio::test]
    async fn test_rollback() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let script_path = temp_dir.path().join("test.lua");
        tokio::fs::write(&script_path, "return 1").await.unwrap();

        let config = HotReloadConfig::new().watch_dir(temp_dir.path());
        let engine: Arc<dyn ScriptEngine> = Arc::new(MockEngine);
        
        let manager = HotReloadManager::new(config, engine).unwrap();
        manager.load_file(&script_path).await.unwrap();

        // Update the script using registry.update (simulating hot-reload)
        let new_version = manager.registry.update("test", "return 2").unwrap();
        assert_eq!(new_version, 2);

        let entry = manager.get_script("test").unwrap();
        assert_eq!(entry.source, "return 2");
        assert_eq!(entry.version, 2);

        // Rollback
        assert!(manager.rollback("test"));
        
        let entry = manager.get_script("test").unwrap();
        // Version increases to 3 after rollback (new version created from history)
        assert_eq!(entry.version, 3);
        // Content should be reverted to "return 1"
        assert_eq!(entry.source, "return 1");
    }
}
