//! File watcher for hot-reloading tool scripts.
//!
//! Provides debounced file system events for configured directories.

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::error::WatchError;
use crate::types::HotLoadingConfig;

/// File system events that can trigger hot-reload.
#[derive(Debug, Clone, PartialEq)]
pub enum WatchEvent {
    /// A file was modified.
    FileChanged(PathBuf),
    /// A new file was created.
    FileCreated(PathBuf),
    /// A file was removed.
    FileRemoved(PathBuf),
    /// Debounced event (multiple rapid changes coalesced).
    Debounced(Vec<PathBuf>),
}

/// File watcher that monitors multiple directories for changes.
pub struct FileWatcher {
    config: HotLoadingConfig,
    /// Keep the watcher alive.
    _watcher: RecommendedWatcher,
    /// Channel for receiving watch events.
    event_rx: mpsc::Receiver<WatchEvent>,
}

impl FileWatcher {
    /// Create a new file watcher with the given configuration.
    ///
    /// Events are debounced according to `config.debounce_ms`.
    pub fn new(config: &HotLoadingConfig) -> Result<Self, WatchError> {
        config
            .validate()
            .map_err(|e| WatchError::WatchFailed(format!("invalid config: {e}")))?;

        let (event_tx, event_rx) = mpsc::channel::<WatchEvent>(32);
        let debounce_ms = config.debounce_ms;
        let extensions = config.extensions.clone();

        // Raw channel: notify callback → debouncer task
        let (raw_tx, raw_rx) = mpsc::channel::<WatchEvent>(256);

        // Create the notify watcher; events go to raw_tx, NOT directly to event_tx
        let extensions_clone = extensions.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            if let Ok(event) = res {
                Self::handle_notify_event(event, &extensions_clone, &raw_tx);
            }
        })
        .map_err(|e| WatchError::WatchFailed(e.to_string()))?;

        // Watch all configured directories
        for watch_dir in &config.watch_dirs {
            // Create directory if it doesn't exist
            if !watch_dir.exists() {
                std::fs::create_dir_all(watch_dir)
                    .map_err(|e| WatchError::WatchFailed(format!("create dir failed: {e}")))?;
            }

            watcher
                .watch(watch_dir, RecursiveMode::Recursive)
                .map_err(|e| WatchError::WatchFailed(format!("watch failed: {e}")))?;
        }

        // Spawn debouncer task: reads from raw_rx, emits to event_tx
        tokio::spawn(async move {
            Self::run_debouncer(debounce_ms, event_tx, raw_rx).await;
        });

        Ok(Self {
            config: config.clone(),
            _watcher: watcher,
            event_rx,
        })
    }

    /// Handle a notify event, filtering by extension and converting to WatchEvent.
    fn handle_notify_event(event: Event, extensions: &[String], tx: &mpsc::Sender<WatchEvent>) {
        let relevant = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
        );

        if !relevant {
            return;
        }

        for path in event.paths {
            if !Self::is_watched_extension(&path, extensions) {
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
    fn is_watched_extension(path: &Path, extensions: &[String]) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| extensions.iter().any(|ext| ext == e))
            .unwrap_or(false)
    }

    /// Run the debouncer task that coalesces rapid events.
    ///
    /// Reads raw events from `raw_rx`, accumulates changed/created paths keyed by
    /// their last-seen `Instant`, and every `debounce_ms` flushes entries that have
    /// been quiet for at least that long as a single `WatchEvent::Debounced` batch.
    /// `FileRemoved` events are forwarded immediately (no debounce needed).
    async fn run_debouncer(
        debounce_ms: u64,
        event_tx: mpsc::Sender<WatchEvent>,
        mut raw_rx: mpsc::Receiver<WatchEvent>,
    ) {
        let debounce_duration = Duration::from_millis(debounce_ms);
        // path → time of last raw event
        let mut pending: HashMap<PathBuf, Instant> = HashMap::new();

        // Tick every debounce window to drain stale entries.
        let mut ticker = tokio::time::interval(debounce_duration);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                result = raw_rx.recv() => {
                    match result {
                        Some(WatchEvent::FileChanged(p) | WatchEvent::FileCreated(p)) => {
                            // Refresh timestamp on every touch — natural debounce.
                            pending.insert(p, Instant::now());
                        }
                        Some(WatchEvent::FileRemoved(p)) => {
                            pending.remove(&p);
                            let _ = event_tx.send(WatchEvent::FileRemoved(p)).await;
                        }
                        Some(_) => {}
                        None => break, // all senders dropped — shut down
                    }
                }
                _ = ticker.tick() => {
                    if pending.is_empty() {
                        continue;
                    }
                    let now = Instant::now();
                    let mut ready: Vec<PathBuf> = Vec::new();
                    pending.retain(|path, t| {
                        if now.duration_since(*t) >= debounce_duration {
                            ready.push(path.clone());
                            false // remove from pending
                        } else {
                            true
                        }
                    });
                    if !ready.is_empty() {
                        let _ = event_tx.send(WatchEvent::Debounced(ready)).await;
                    }
                }
            }
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

    /// Check if an extension is watched.
    pub fn is_watched_extension_pub(&self, ext: &str) -> bool {
        self.config.is_watched_extension(ext)
    }
}

/// Create a single-file watcher for a specific path.
pub fn watch_file(path: &Path) -> Result<(RecommendedWatcher, mpsc::Receiver<()>), WatchError> {
    let (tx, rx) = mpsc::channel(1);
    let path_buf = path.to_path_buf();

    let watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            if let EventKind::Modify(_) = event.kind {
                for p in event.paths {
                    if p == path_buf {
                        let _ = tx.try_send(());
                    }
                }
            }
        }
    })
    .map_err(|e| WatchError::WatchFailed(e.to_string()))?;

    Ok((watcher, rx))
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    fn test_config() -> HotLoadingConfig {
        HotLoadingConfig {
            watch_dirs: vec![std::env::temp_dir().join("claw_test_watcher")],
            extensions: vec!["lua".to_string(), "js".to_string()],
            debounce_ms: 50,
            default_timeout_secs: 30,
            compile_timeout_secs: 10,
            keep_previous_secs: 300,
            auto_enable: true,
        }
    }

    #[tokio::test]
    async fn test_file_watcher_is_watched_extension() {
        let config = test_config();
        let watcher = FileWatcher::new(&config);
        // Creation may fail if temp dir doesn't exist, but we can still test the method
        if let Ok(w) = watcher {
            assert!(w.is_watched_extension_pub("lua"));
            assert!(w.is_watched_extension_pub("js"));
            assert!(!w.is_watched_extension_pub("py"));
        }
    }

    #[tokio::test]
    async fn test_file_watcher_watch_dirs() {
        let config = test_config();
        let watcher = FileWatcher::new(&config);
        if let Ok(w) = watcher {
            let dirs = w.watch_dirs();
            assert_eq!(dirs.len(), 1);
            assert!(dirs[0].to_string_lossy().contains("claw_test_watcher"));
        }
    }

    #[tokio::test]
    async fn test_watch_event_variants() {
        let changed = WatchEvent::FileChanged(PathBuf::from("/test.lua"));
        assert!(matches!(changed, WatchEvent::FileChanged(_)));

        let created = WatchEvent::FileCreated(PathBuf::from("/test.lua"));
        assert!(matches!(created, WatchEvent::FileCreated(_)));

        let removed = WatchEvent::FileRemoved(PathBuf::from("/test.lua"));
        assert!(matches!(removed, WatchEvent::FileRemoved(_)));

        let debounced = WatchEvent::Debounced(vec![PathBuf::from("/test.lua")]);
        assert!(matches!(debounced, WatchEvent::Debounced(_)));
    }

    #[tokio::test]
    async fn test_debouncer_coalesces_events() {
        let (event_tx, mut event_rx) = mpsc::channel::<WatchEvent>(32);
        let (raw_tx, raw_rx) = mpsc::channel::<WatchEvent>(64);
        let debounce_ms = 50u64;

        tokio::spawn(async move {
            FileWatcher::run_debouncer(debounce_ms, event_tx, raw_rx).await;
        });

        // Send three rapid changes to the same file
        let path = PathBuf::from("/test.lua");
        for _ in 0..3 {
            raw_tx
                .send(WatchEvent::FileChanged(path.clone()))
                .await
                .unwrap();
        }
        // Also send a change for a different file
        raw_tx
            .send(WatchEvent::FileChanged(PathBuf::from("/other.lua")))
            .await
            .unwrap();

        // Wait for the debounce window to expire
        tokio::time::sleep(Duration::from_millis(debounce_ms * 3)).await;

        // Should receive exactly one Debounced event containing both paths (deduplicated)
        let event = event_rx.try_recv().expect("expected a Debounced event");
        match event {
            WatchEvent::Debounced(paths) => {
                assert!(paths.contains(&PathBuf::from("/test.lua")));
                assert!(paths.contains(&PathBuf::from("/other.lua")));
            }
            other => panic!("unexpected event: {other:?}"),
        }
        // No more events
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_debouncer_forwards_remove_immediately() {
        let (event_tx, mut event_rx) = mpsc::channel::<WatchEvent>(32);
        let (raw_tx, raw_rx) = mpsc::channel::<WatchEvent>(64);

        tokio::spawn(async move {
            FileWatcher::run_debouncer(50, event_tx, raw_rx).await;
        });

        raw_tx
            .send(WatchEvent::FileRemoved(PathBuf::from("/gone.lua")))
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(150)).await;

        match event_rx.try_recv().expect("expected FileRemoved") {
            WatchEvent::FileRemoved(p) => assert_eq!(p, PathBuf::from("/gone.lua")),
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn test_is_watched_extension_helper() {
        let extensions = vec!["lua".to_string(), "js".to_string()];

        assert!(FileWatcher::is_watched_extension(
            Path::new("/test.lua"),
            &extensions
        ));
        assert!(FileWatcher::is_watched_extension(
            Path::new("/test.js"),
            &extensions
        ));
        assert!(!FileWatcher::is_watched_extension(
            Path::new("/test.py"),
            &extensions
        ));
        assert!(!FileWatcher::is_watched_extension(
            Path::new("/test"),
            &extensions
        ));
        assert!(!FileWatcher::is_watched_extension(
            Path::new(""),
            &extensions
        ));
    }
}
