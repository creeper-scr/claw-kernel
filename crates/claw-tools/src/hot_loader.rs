use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::{error::WatchError, types::HotLoadingConfig};

/// File watcher for hot-loading tool scripts.
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

        // Start watching the configured directory.
        let watch_path = PathBuf::from(&config.watch_dir);
        watcher
            .watch(&watch_path, RecursiveMode::Recursive)
            .map_err(|e| WatchError::WatchFailed(e.to_string()))?;

        // Spawn background task: read events, apply debounce, call on_change.
        let debounce_ms = config.debounce_ms;
        tokio::spawn(async move {
            while let Some(path) = event_rx.recv().await {
                // Coalesce rapid events within the debounce window.
                tokio::time::sleep(Duration::from_millis(debounce_ms)).await;
                while event_rx.try_recv().is_ok() {}
                on_change(path);
            }
        });

        Ok(Self {
            config,
            _watcher: watcher,
            _event_tx: event_tx,
        })
    }

    /// Return the watched directory path as configured.
    pub fn watched_path(&self) -> &str {
        &self.config.watch_dir
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

    fn default_config() -> HotLoadingConfig {
        HotLoadingConfig::default()
    }

    #[test]
    fn test_hot_loader_default_config() {
        let config = default_config();
        assert_eq!(config.watch_dir, "tools");
        assert_eq!(config.extensions, vec!["lua"]);
        assert_eq!(config.debounce_ms, 50);
        assert_eq!(config.default_timeout_secs, 30);
    }

    #[test]
    fn test_hot_loader_is_watched_extension() {
        // Build a config pointing at an existing tmp dir so watch() succeeds.
        let tmp = std::env::temp_dir();
        let config = HotLoadingConfig {
            watch_dir: tmp.to_string_lossy().into_owned(),
            extensions: vec!["lua".to_string(), "js".to_string()],
            debounce_ms: 50,
            default_timeout_secs: 30,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let loader = rt
            .block_on(async {
                // We need a tokio context for the spawn inside HotLoader::new.
                HotLoader::new(config, |_| {})
            })
            .expect("HotLoader::new should succeed");

        assert!(loader.is_watched_extension("lua"));
        assert!(loader.is_watched_extension("js"));
    }

    #[test]
    fn test_hot_loader_non_watched_extension() {
        let tmp = std::env::temp_dir();
        let config = HotLoadingConfig {
            watch_dir: tmp.to_string_lossy().into_owned(),
            extensions: vec!["lua".to_string()],
            debounce_ms: 50,
            default_timeout_secs: 30,
        };
        let rt = tokio::runtime::Runtime::new().unwrap();
        let loader = rt
            .block_on(async { HotLoader::new(config, |_| {}) })
            .expect("HotLoader::new should succeed");

        assert!(!loader.is_watched_extension("py"));
        assert!(!loader.is_watched_extension("rs"));
        assert!(!loader.is_watched_extension(""));
    }

    #[tokio::test]
    async fn test_hot_loader_create_with_tmpdir() {
        let tmp = std::env::temp_dir();
        let config = HotLoadingConfig {
            watch_dir: tmp.to_string_lossy().into_owned(),
            extensions: vec!["lua".to_string()],
            debounce_ms: 50,
            default_timeout_secs: 30,
        };
        let loader = HotLoader::new(config, move |_path| {
            // callback — just mark that it's callable
        });
        // Should not panic or error.
        assert!(loader.is_ok(), "HotLoader::new with tmpdir should succeed");
    }

    #[tokio::test]
    async fn test_hot_loader_watched_path() {
        let tmp = std::env::temp_dir();
        let dir_str = tmp.to_string_lossy().into_owned();
        let config = HotLoadingConfig {
            watch_dir: dir_str.clone(),
            extensions: vec!["lua".to_string()],
            debounce_ms: 50,
            default_timeout_secs: 30,
        };
        let loader = HotLoader::new(config, |_| {}).expect("should create");
        assert_eq!(loader.watched_path(), &dir_str);
    }
}
