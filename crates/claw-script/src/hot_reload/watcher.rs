//! File watcher for script hot-reloading.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::hot_reload::config::HotReloadConfig;
use crate::hot_reload::events::{ScriptEvent, ScriptEventBus};

/// File system events that can trigger hot-reload.
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// A file was modified.
    FileChanged(PathBuf),
    /// A new file was created.
    FileCreated(PathBuf),
    /// A file was removed.
    FileRemoved(PathBuf),
    /// Debounced batch of events.
    Debounced(Vec<PathBuf>),
}

/// File watcher that monitors script directories for changes.
pub struct ScriptWatcher {
    config: HotReloadConfig,
    _watcher: RecommendedWatcher,
    event_rx: mpsc::Receiver<WatchEvent>,
}

impl ScriptWatcher {
    /// Create a new script watcher with the given configuration.
    pub fn new(config: HotReloadConfig, event_bus: Option<Arc<ScriptEventBus>>) -> Result<Self, crate::error::ScriptError> {
        config.validate().map_err(|e| {
            crate::error::ScriptError::Runtime(format!("Invalid config: {}", e))
        })?;

        // raw_tx/raw_rx: notify watcher → debouncer (raw, high-volume)
        let (raw_tx, raw_rx) = mpsc::channel::<WatchEvent>(256);
        // event_tx/event_rx: debouncer → consumer (batched, debounced)
        let (event_tx, event_rx) = mpsc::channel::<WatchEvent>(128);

        let debounce_delay = config.debounce_delay;
        let extensions = config.extensions.clone();

        // Create the notify watcher — sends raw events to raw_tx
        let watcher_extensions = extensions.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                Self::handle_notify_event(event, &watcher_extensions, &raw_tx);
            }
        })
        .map_err(|e| crate::error::ScriptError::Runtime(format!("Failed to create watcher: {}", e)))?;

        // Watch all configured directories
        let recursive_mode = if config.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        for watch_dir in &config.watch_dirs {
            // Create directory if it doesn't exist
            if !watch_dir.exists() {
                std::fs::create_dir_all(watch_dir).map_err(|e| {
                    crate::error::ScriptError::Runtime(format!(
                        "Failed to create directory {:?}: {}",
                        watch_dir, e
                    ))
                })?;
            }

            watcher
                .watch(watch_dir, recursive_mode)
                .map_err(|e| crate::error::ScriptError::Runtime(format!("Watch failed: {}", e)))?;
        }

        // Spawn debouncer: reads from raw_rx, sends batched WatchEvent::Debounced to event_tx
        tokio::spawn(async move {
            Self::run_debouncer(debounce_delay, raw_rx, event_tx).await;
        });

        // Emit started event
        if let Some(bus) = event_bus {
            let _ = bus.emit(ScriptEvent::Started {
                directories: config.watch_dirs.clone(),
            });
        }

        Ok(Self {
            config,
            _watcher: watcher,
            event_rx,
        })
    }

    /// Handle a raw notify event.
    fn handle_notify_event(
        event: Event,
        extensions: &std::collections::HashSet<String>,
        tx: &mpsc::Sender<WatchEvent>,
    ) {
        let relevant = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        );

        if !relevant {
            return;
        }

        for path in event.paths {
            if !Self::has_watched_extension(&path, extensions) {
                continue;
            }

            let watch_event = match event.kind {
                EventKind::Create(_) => WatchEvent::FileCreated(path),
                EventKind::Modify(_) => WatchEvent::FileChanged(path),
                EventKind::Remove(_) => WatchEvent::FileRemoved(path),
                _ => continue,
            };

            let _ = tx.try_send(watch_event);
        }
    }

    /// Check if a file has a watched extension.
    fn has_watched_extension(path: &Path, extensions: &std::collections::HashSet<String>) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| extensions.contains(e))
            .unwrap_or(false)
    }

    /// Run the debouncer task.
    ///
    /// Reads raw events from `raw_rx`, accumulates them for `debounce_delay`,
    /// then emits a single `WatchEvent::Debounced` batch to `event_tx`.
    /// Tracks the last event type per path (a later Remove wins over an earlier Create).
    pub(crate) async fn run_debouncer(
        debounce_delay: Duration,
        mut raw_rx: mpsc::Receiver<WatchEvent>,
        event_tx: mpsc::Sender<WatchEvent>,
    ) {
        // pending maps path → last event kind (so we don't reload a removed file)
        let mut pending: HashMap<PathBuf, WatchEvent> = HashMap::new();

        loop {
            if pending.is_empty() {
                // Block until at least one event arrives
                match raw_rx.recv().await {
                    Some(event) => Self::accumulate_event(&mut pending, event),
                    None => break, // channel closed, debouncer exits
                }
            }

            // Drain additional events within the debounce window
            let deadline = tokio::time::Instant::now() + debounce_delay;
            loop {
                match tokio::time::timeout_at(deadline, raw_rx.recv()).await {
                    Ok(Some(event)) => Self::accumulate_event(&mut pending, event),
                    Ok(None) => {
                        // Channel closed — flush remaining events and exit
                        if !pending.is_empty() {
                            let paths: Vec<PathBuf> = pending.keys().cloned().collect();
                            let _ = event_tx.send(WatchEvent::Debounced(paths)).await;
                        }
                        return;
                    }
                    Err(_timeout) => break, // debounce window elapsed
                }
            }

            // Flush the accumulated batch
            if !pending.is_empty() {
                let paths: Vec<PathBuf> = pending.keys().cloned().collect();
                let _ = event_tx.send(WatchEvent::Debounced(paths)).await;
                pending.clear();
            }
        }
    }

    /// Accumulate a raw event into the pending map.
    ///
    /// Rules:
    /// - Remove always wins (a file removed after being created = deleted)
    /// - FileChanged does not overwrite an earlier FileCreated
    pub(crate) fn accumulate_event(pending: &mut HashMap<PathBuf, WatchEvent>, event: WatchEvent) {
        match event {
            WatchEvent::FileCreated(p) => {
                pending.insert(p.clone(), WatchEvent::FileCreated(p));
            }
            WatchEvent::FileChanged(p) => {
                // Don't downgrade a pending Create to Changed
                pending.entry(p.clone()).or_insert(WatchEvent::FileChanged(p));
            }
            WatchEvent::FileRemoved(p) => {
                pending.insert(p.clone(), WatchEvent::FileRemoved(p));
            }
            _ => {}
        }
    }

    /// Receive the next watch event.
    pub async fn recv(&mut self) -> Option<WatchEvent> {
        self.event_rx.recv().await
    }

    /// Try to receive a watch event without blocking.
    pub fn try_recv(&mut self) -> Result<WatchEvent, mpsc::error::TryRecvError> {
        self.event_rx.try_recv()
    }

    /// Get the watched directories.
    pub fn watch_dirs(&self) -> &[PathBuf] {
        &self.config.watch_dirs
    }

    /// Get the config.
    pub fn config(&self) -> &HotReloadConfig {
        &self.config
    }
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_has_watched_extension() {
        let extensions = ["lua".to_string(), "js".to_string()]
            .into_iter()
            .collect();

        assert!(ScriptWatcher::has_watched_extension(
            Path::new("test.lua"),
            &extensions
        ));
        assert!(ScriptWatcher::has_watched_extension(
            Path::new("/path/to/test.js"),
            &extensions
        ));
        assert!(!ScriptWatcher::has_watched_extension(
            Path::new("test.py"),
            &extensions
        ));
        assert!(!ScriptWatcher::has_watched_extension(
            Path::new("test"),
            &extensions
        ));
    }

    #[tokio::test]
    async fn test_watcher_creation() {
        let temp_dir = TempDir::new().unwrap();
        let config = HotReloadConfig::new().watch_dir(temp_dir.path());

        let result = ScriptWatcher::new(config, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_watch_event_variants() {
        let changed = WatchEvent::FileChanged(PathBuf::from("/test.lua"));
        assert!(matches!(changed, WatchEvent::FileChanged(_)));

        let created = WatchEvent::FileCreated(PathBuf::from("/test.lua"));
        assert!(matches!(created, WatchEvent::FileCreated(_)));

        let removed = WatchEvent::FileRemoved(PathBuf::from("/test.lua"));
        assert!(matches!(removed, WatchEvent::FileRemoved(_)));

        let debounced = WatchEvent::Debounced(vec![PathBuf::from("/test.lua")]);
        assert!(matches!(debounced, WatchEvent::Debounced(_)));
    }

    #[test]
    fn test_accumulate_remove_wins_over_create() {
        let mut pending = HashMap::new();
        let p = PathBuf::from("/test.lua");

        ScriptWatcher::accumulate_event(&mut pending, WatchEvent::FileCreated(p.clone()));
        ScriptWatcher::accumulate_event(&mut pending, WatchEvent::FileRemoved(p.clone()));
        assert!(matches!(pending[&p], WatchEvent::FileRemoved(_)));
    }

    #[test]
    fn test_accumulate_create_not_overwritten_by_change() {
        let mut pending = HashMap::new();
        let p = PathBuf::from("/test.lua");

        ScriptWatcher::accumulate_event(&mut pending, WatchEvent::FileCreated(p.clone()));
        ScriptWatcher::accumulate_event(&mut pending, WatchEvent::FileChanged(p.clone()));
        assert!(matches!(pending[&p], WatchEvent::FileCreated(_)));
    }

    #[tokio::test]
    async fn test_debouncer_batches_events() {
        let debounce_delay = Duration::from_millis(30);
        let (raw_tx, raw_rx) = mpsc::channel::<WatchEvent>(16);
        let (event_tx, mut event_rx) = mpsc::channel::<WatchEvent>(16);

        tokio::spawn(ScriptWatcher::run_debouncer(debounce_delay, raw_rx, event_tx));

        let p1 = PathBuf::from("a.lua");
        let p2 = PathBuf::from("b.lua");
        let p3 = PathBuf::from("c.lua");
        raw_tx.send(WatchEvent::FileChanged(p1.clone())).await.unwrap();
        raw_tx.send(WatchEvent::FileChanged(p2.clone())).await.unwrap();
        raw_tx.send(WatchEvent::FileChanged(p3.clone())).await.unwrap();

        // After debounce window, we should get a single Debounced batch
        let event = tokio::time::timeout(Duration::from_millis(200), event_rx.recv())
            .await
            .expect("timed out waiting for debounced event")
            .expect("channel closed");

        match event {
            WatchEvent::Debounced(paths) => {
                assert_eq!(paths.len(), 3);
                assert!(paths.contains(&p1));
                assert!(paths.contains(&p2));
                assert!(paths.contains(&p3));
            }
            other => panic!("expected Debounced, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_debouncer_deduplicates_same_path() {
        let debounce_delay = Duration::from_millis(30);
        let (raw_tx, raw_rx) = mpsc::channel::<WatchEvent>(16);
        let (event_tx, mut event_rx) = mpsc::channel::<WatchEvent>(16);

        tokio::spawn(ScriptWatcher::run_debouncer(debounce_delay, raw_rx, event_tx));

        let p = PathBuf::from("same.lua");
        raw_tx.send(WatchEvent::FileChanged(p.clone())).await.unwrap();
        raw_tx.send(WatchEvent::FileChanged(p.clone())).await.unwrap();
        raw_tx.send(WatchEvent::FileChanged(p.clone())).await.unwrap();

        let event = tokio::time::timeout(Duration::from_millis(200), event_rx.recv())
            .await
            .expect("timed out")
            .expect("channel closed");

        match event {
            WatchEvent::Debounced(paths) => {
                assert_eq!(paths.len(), 1, "same path should be deduplicated");
                assert_eq!(paths[0], p);
            }
            other => panic!("expected Debounced, got {:?}", other),
        }
    }
}
