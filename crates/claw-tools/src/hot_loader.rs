use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::{error::WatchError, types::HotLoadingConfig};

// Note: This module is kept for backward compatibility.
// For new code, use the hot_reload module which provides more comprehensive
// hot-reloading capabilities including atomic swaps and version management.

/// File watcher for hot-loading tool scripts.
///
/// # Deprecated
///
/// This struct is deprecated in favor of the `hot_reload` module which provides
/// more comprehensive hot-reloading capabilities:
///
/// - [`FileWatcher`](crate::hot_reload::FileWatcher) for file watching
/// - [`HotReloadProcessor`](crate::hot_reload::HotReloadProcessor) for event processing
/// - [`VersionedModule`](crate::hot_reload::VersionedModule) for atomic module swapping
/// - [`VersionedToolSet`](crate::hot_reload::VersionedToolSet) for versioned tool collections
///
/// # Migration
///
/// ```rust,ignore
/// // Old API (deprecated)
/// let loader = HotLoader::new(config, |path| { ... })?;
///
/// // New API (recommended)
/// use claw_tools::hot_reload::{FileWatcher, HotReloadProcessor};
///
/// let mut watcher = FileWatcher::new(&config)?;
/// let processor = HotReloadProcessor::new(registry, config);
/// // ... set up event channel ...
/// ```
#[deprecated(
    since = "0.1.0",
    note = "Use the hot_reload module: FileWatcher, HotReloadProcessor, VersionedModule, VersionedToolSet"
)]
///
/// Watches a directory and calls the provided callback when a relevant file
/// changes. Rapid filesystem events are coalesced via a configurable debounce
/// window (default 50 ms).
pub struct HotLoader {
    config: HotLoadingConfig,
    /// Keep the watcher alive; dropping it would stop event delivery.
    _watcher: RecommendedWatcher,
    /// Channel sender (kept alive so the spawned task's receiver stays open).
    _event_tx: mpsc::Sender<PathBuf>,
}

#[allow(deprecated)]
impl HotLoader {
    /// Create a new `HotLoader` with the given config.
    ///
    /// `on_change` is called with the changed file path whenever a relevant
    /// script file is created, modified, or removed.
    pub fn new<F>(config: HotLoadingConfig, on_change: F) -> Result<Self, WatchError>
    where
        F: Fn(PathBuf) + Send + 'static,
    {
        let (event_tx, mut event_rx) = mpsc::channel::<PathBuf>(32);
        let tx_clone = event_tx.clone();

        // Build the notify watcher using the functional API (notify 6.x).
        let extensions: Vec<String> = config.extensions.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                // Only react to create / modify / remove events.
                let relevant = matches!(
                    event.kind,
                    EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
                );
                if !relevant {
                    return;
                }
                for path in &event.paths {
                    // Filter by watched extensions.
                    let ext_ok = path
                        .extension()
                        .and_then(|e| e.to_str())
                        .map(|e| extensions.iter().any(|w| w == e))
                        .unwrap_or(false);
                    if ext_ok {
                        let _ = tx_clone.blocking_send(path.clone());
                    }
                }
            }
        })
        .map_err(|e| WatchError::WatchFailed(e.to_string()))?;

        // Start watching the configured directories.
        // ISSUE-001 fix: Ensure watch directories exist before watching.
        for watch_dir in &config.watch_dirs {
            fs::create_dir_all(watch_dir).map_err(|e| {
                WatchError::WatchFailed(format!("Failed to create watch dir: {}", e))
            })?;
            watcher
                .watch(watch_dir, RecursiveMode::Recursive)
                .map_err(|e| WatchError::WatchFailed(e.to_string()))?;
        }

        // Spawn background task: read events, apply debounce, call on_change.
        // Uses a HashSet to track all unique changed paths within the debounce window,
        // ensuring no file change events are lost.
        let debounce_ms = config.debounce_ms;
        tokio::spawn(async move {
            let mut pending_paths: HashSet<PathBuf> = HashSet::new();
            let debounce_duration = Duration::from_millis(debounce_ms);

            loop {
                // Wait for the first event
                match event_rx.recv().await {
                    Some(path) => {
                        pending_paths.insert(path);
                    }
                    None => break, // Channel closed
                }

                // Collect all events within the debounce window
                let debounce_deadline = tokio::time::Instant::now() + debounce_duration;
                loop {
                    let timeout =
                        debounce_deadline.saturating_duration_since(tokio::time::Instant::now());
                    match tokio::time::timeout(timeout, event_rx.recv()).await {
                        Ok(Some(path)) => {
                            pending_paths.insert(path);
                        }
                        Ok(None) => break, // Channel closed
                        Err(_) => break,   // Debounce window expired
                    }
                }

                // Process all unique changed paths
                for path in pending_paths.drain() {
                    on_change(path);
                }
            }
        });

        Ok(Self {
            config,
            _watcher: watcher,
            _event_tx: event_tx,
        })
    }

    /// Return the first watched directory path as configured.
    pub fn watched_path(&self) -> &str {
        self.config
            .watch_dirs
            .first()
            .map(|p| p.to_str().unwrap_or(""))
            .unwrap_or("")
    }

    /// Check if a file extension is in the watched list.
    pub fn is_watched_extension(&self, ext: &str) -> bool {
        self.config.extensions.iter().any(|e| e == ext)
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hot_reload::FileWatcher;

    fn default_config() -> HotLoadingConfig {
        HotLoadingConfig::default()
    }

    #[test]
    fn test_hot_loader_default_config() {
        let config = default_config();
        assert_eq!(config.watch_dirs, vec![PathBuf::from("tools")]);
        assert_eq!(config.extensions, vec!["lua"]);
        assert_eq!(config.debounce_ms, 50);
        assert_eq!(config.default_timeout_secs, 30);
    }

    #[tokio::test]
    async fn test_hot_loader_is_watched_extension() {
        // Build a config pointing at an existing tmp dir so watch() succeeds.
        let tmp = std::env::temp_dir();
        let config = HotLoadingConfig {
            watch_dirs: vec![tmp],
            extensions: vec!["lua".to_string(), "js".to_string()],
            ..Default::default()
        };
        // Use new FileWatcher API instead of deprecated HotLoader
        let watcher = FileWatcher::new(&config).expect("FileWatcher::new should succeed");

        assert!(watcher.is_watched_extension_pub("lua"));
        assert!(watcher.is_watched_extension_pub("js"));
    }

    #[tokio::test]
    async fn test_hot_loader_non_watched_extension() {
        let tmp = std::env::temp_dir();
        let config = HotLoadingConfig {
            watch_dirs: vec![tmp],
            extensions: vec!["lua".to_string()],
            ..Default::default()
        };
        // Use new FileWatcher API instead of deprecated HotLoader
        let watcher = FileWatcher::new(&config).expect("FileWatcher::new should succeed");

        assert!(!watcher.is_watched_extension_pub("py"));
        assert!(!watcher.is_watched_extension_pub("rs"));
        assert!(!watcher.is_watched_extension_pub(""));
    }

    #[tokio::test]
    async fn test_hot_loader_create_with_tmpdir() {
        let tmp = std::env::temp_dir();
        let config = HotLoadingConfig {
            watch_dirs: vec![tmp],
            extensions: vec!["lua".to_string()],
            ..Default::default()
        };
        // Use new FileWatcher API instead of deprecated HotLoader
        let watcher = FileWatcher::new(&config);
        // Should not panic or error.
        assert!(
            watcher.is_ok(),
            "FileWatcher::new with tmpdir should succeed"
        );
    }

    #[tokio::test]
    async fn test_hot_loader_watched_path() {
        let tmp = std::env::temp_dir();
        let dir_str = tmp.to_string_lossy().into_owned();
        let config = HotLoadingConfig {
            watch_dirs: vec![tmp],
            extensions: vec!["lua".to_string()],
            ..Default::default()
        };
        // Use new FileWatcher API instead of deprecated HotLoader
        let watcher = FileWatcher::new(&config).expect("should create");
        // Check first watched directory matches
        let dirs = watcher.watch_dirs();
        assert_eq!(dirs.len(), 1);
        assert_eq!(dirs[0].to_string_lossy().into_owned(), dir_str);
    }
}
