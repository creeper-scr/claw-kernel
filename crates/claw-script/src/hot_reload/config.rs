//! Configuration for script hot-reloading.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::types::EngineType;

/// Configuration for script hot-reloading.
#[derive(Debug, Clone)]
pub struct HotReloadConfig {
    /// Directories to watch for script files.
    pub watch_dirs: Vec<PathBuf>,
    /// File extensions to watch (e.g., ["lua", "js", "ts"]).
    pub extensions: HashSet<String>,
    /// Debounce delay for file system events.
    pub debounce_delay: Duration,
    /// Maximum number of script versions to keep in history.
    pub max_history_size: usize,
    /// Enable auto-reload on file change.
    pub auto_reload: bool,
    /// Validate scripts before hot-reloading.
    pub validate_before_reload: bool,
    /// Engine type filter (None = all engines).
    pub engine_filter: Option<EngineType>,
    /// Allow loading scripts from subdirectories.
    pub recursive: bool,
}

impl Default for HotReloadConfig {
    fn default() -> Self {
        let mut extensions = HashSet::new();
        extensions.insert("lua".to_string());
        #[cfg(feature = "engine-v8")]
        {
            extensions.insert("js".to_string());
            extensions.insert("ts".to_string());
        }

        Self {
            watch_dirs: vec![PathBuf::from("./scripts")],
            extensions,
            debounce_delay: Duration::from_millis(50),
            max_history_size: 5,
            auto_reload: true,
            validate_before_reload: true,
            engine_filter: None,
            recursive: true,
        }
    }
}

impl HotReloadConfig {
    /// Create a new config with default values.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a directory to watch.
    pub fn watch_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.watch_dirs.push(path.into());
        self
    }

    /// Add a file extension to watch.
    pub fn extension(mut self, ext: impl Into<String>) -> Self {
        self.extensions.insert(ext.into());
        self
    }

    /// Set the debounce delay.
    pub fn debounce_delay(mut self, delay: Duration) -> Self {
        self.debounce_delay = delay;
        self
    }

    /// Set the maximum history size.
    pub fn max_history_size(mut self, size: usize) -> Self {
        self.max_history_size = size;
        self
    }

    /// Enable or disable auto-reload.
    pub fn auto_reload(mut self, enable: bool) -> Self {
        self.auto_reload = enable;
        self
    }

    /// Enable or disable validation before reload.
    pub fn validate_before_reload(mut self, enable: bool) -> Self {
        self.validate_before_reload = enable;
        self
    }

    /// Filter by engine type.
    pub fn engine_filter(mut self, engine: EngineType) -> Self {
        self.engine_filter = Some(engine);
        self
    }

    /// Set recursive watching.
    pub fn recursive(mut self, enable: bool) -> Self {
        self.recursive = enable;
        self
    }

    /// Check if a file extension is watched.
    pub fn is_watched_extension(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| self.extensions.contains(e))
            .unwrap_or(false)
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.watch_dirs.is_empty() {
            return Err("At least one watch directory must be specified".to_string());
        }

        if self.extensions.is_empty() {
            return Err("At least one extension must be specified".to_string());
        }

        if self.max_history_size == 0 {
            return Err("max_history_size must be greater than 0".to_string());
        }

        Ok(())
    }

    /// Infer engine type from file extension.
    pub fn engine_type_from_extension(&self, path: &Path) -> Option<EngineType> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| match ext {
                "lua" => Some(EngineType::Lua),
                #[cfg(feature = "engine-v8")]
                "js" | "ts" | "mjs" => Some(EngineType::JavaScript),
                _ => None,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = HotReloadConfig::default();
        assert_eq!(config.watch_dirs.len(), 1);
        assert!(config.extensions.contains("lua"));
        assert_eq!(config.debounce_delay, Duration::from_millis(50));
        assert_eq!(config.max_history_size, 5);
        assert!(config.auto_reload);
        assert!(config.validate_before_reload);
    }

    #[test]
    fn test_builder_pattern() {
        let config = HotReloadConfig::new()
            .watch_dir("/custom/scripts")
            .extension("py")
            .debounce_delay(Duration::from_millis(100))
            .max_history_size(10)
            .auto_reload(false)
            .validate_before_reload(false);

        assert_eq!(config.watch_dirs.len(), 2);
        assert!(config.extensions.contains("py"));
        assert_eq!(config.debounce_delay, Duration::from_millis(100));
        assert_eq!(config.max_history_size, 10);
        assert!(!config.auto_reload);
        assert!(!config.validate_before_reload);
    }

    #[test]
    fn test_is_watched_extension() {
        let config = HotReloadConfig::default();
        assert!(config.is_watched_extension(&PathBuf::from("test.lua")));
        assert!(!config.is_watched_extension(&PathBuf::from("test.py")));
        assert!(!config.is_watched_extension(&PathBuf::from("test")));
    }

    #[test]
    fn test_validate_empty_watch_dirs() {
        let config = HotReloadConfig {
            watch_dirs: vec![],
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_empty_extensions() {
        let config = HotReloadConfig {
            extensions: HashSet::new(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_engine_type_from_extension() {
        let config = HotReloadConfig::default();
        assert_eq!(
            config.engine_type_from_extension(&PathBuf::from("test.lua")),
            Some(EngineType::Lua)
        );
        #[cfg(feature = "engine-v8")]
        assert_eq!(
            config.engine_type_from_extension(&PathBuf::from("test.js")),
            Some(EngineType::JavaScript)
        );
        assert_eq!(
            config.engine_type_from_extension(&PathBuf::from("test.py")),
            None
        );
    }
}
