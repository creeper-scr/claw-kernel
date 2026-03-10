//! HotLoader IPC bridge (G-11).
//!
//! Exposes `HotLoaderHandle` — a shareable, runtime-configurable wrapper around
//! [`notify`]'s file-system watcher — so that external clients (Python / TS SDK)
//! can call `tool.watch_dir` / `tool.reload` over IPC and receive
//! `tool/hot_reloaded` push notifications whenever watched scripts change.
//!
//! # Architecture
//!
//! ```text
//! notify watcher ──(sync callback)──► mpsc::Sender<PathBuf>
//!                                          │
//!                                    dispatch_loop (async)
//!                                          │  50 ms debounce
//!                                          ▼
//!                               fan-out to subscribers
//!                               (one per IPC connection)
//!                                          │
//!                                    tool/hot_reloaded  ◄── JSON-RPC push
//! ```

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

use dashmap::{DashMap, DashSet};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

// ─── Inner state ──────────────────────────────────────────────────────────────

struct HotLoaderInner {
    /// The underlying notify watcher. Kept alive for the server lifetime.
    watcher: Mutex<RecommendedWatcher>,
    /// All directories currently being watched (deduplication).
    watched_dirs: DashSet<PathBuf>,
    #[allow(dead_code)]
    /// File extensions to watch (e.g. `["lua", "js"]`).
    extensions: Vec<String>,
    /// Active subscribers: conn_id → notify_tx (raw JSON bytes, no frame prefix).
    subscribers: DashMap<u64, mpsc::Sender<Vec<u8>>>,
    #[allow(dead_code)]
    /// Monotonic counter for subscriber IDs (== connection IDs).
    next_id: AtomicU64,
    /// Channel used by `trigger_reload()` to inject manual events.
    event_tx: mpsc::Sender<PathBuf>,
}

// ─── Public handle ─────────────────────────────────────────────────────────────

/// Shared handle to the server-level hot-loader.
///
/// Cheap to clone (wraps an `Arc`). All clones share the same watcher state
/// and subscriber set.
#[derive(Clone)]
pub struct HotLoaderHandle {
    inner: Arc<HotLoaderInner>,
}

impl HotLoaderHandle {
    /// Create a new `HotLoaderHandle` and spawn the dispatch background task.
    ///
    /// Returns `(handle, task)`. The task drives the debounced fan-out; it
    /// terminates automatically when the handle (and all clones) are dropped.
    ///
    /// # Panics
    /// Panics if the underlying `notify` watcher cannot be initialised (very
    /// rare; only on platforms with broken inotify / FSEvents).
    pub fn new(extensions: Vec<String>) -> (Self, tokio::task::JoinHandle<()>) {
        let (event_tx, event_rx) = mpsc::channel::<PathBuf>(256);

        // Clones sent into the sync notify callback.
        let tx_cb = event_tx.clone();
        let exts_cb = extensions.clone();

        let watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
            let Ok(event) = res else { return };
            if !matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_)) {
                return;
            }
            for path in event.paths {
                let watched = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| exts_cb.iter().any(|x| x.as_str() == e))
                    .unwrap_or(false);
                if watched {
                    let _ = tx_cb.try_send(path);
                }
            }
        })
        .expect("failed to create notify watcher");

        let inner = Arc::new(HotLoaderInner {
            watcher: Mutex::new(watcher),
            watched_dirs: DashSet::new(),
            extensions,
            subscribers: DashMap::new(),
            next_id: AtomicU64::new(0),
            event_tx,
        });

        let inner_task = Arc::clone(&inner);
        let task = tokio::spawn(async move {
            dispatch_loop(inner_task, event_rx).await;
        });

        (Self { inner }, task)
    }

    /// Add `path` to the watched directories (idempotent).
    ///
    /// Creates the directory if it does not exist. After this call, any
    /// `.lua` / `.js` (or whichever extensions were configured) file changes
    /// under `path` will be debounced and delivered to all subscribers as
    /// `tool/hot_reloaded` notifications.
    pub fn watch_dir(&self, path: PathBuf) -> Result<(), String> {
        if self.inner.watched_dirs.contains(&path) {
            return Ok(());
        }
        if !path.exists() {
            std::fs::create_dir_all(&path)
                .map_err(|e| format!("failed to create directory: {}", e))?;
        }
        self.inner
            .watcher
            .lock()
            .map_err(|_| "watcher mutex poisoned".to_string())?
            .watch(&path, RecursiveMode::Recursive)
            .map_err(|e| format!("watch failed: {}", e))?;
        self.inner.watched_dirs.insert(path);
        Ok(())
    }

    /// Manually inject a reload event for `path`.
    ///
    /// Equivalent to the file changing on disk; the 50ms debounce still applies.
    pub async fn trigger_reload(&self, path: PathBuf) -> Result<(), String> {
        self.inner
            .event_tx
            .send(path)
            .await
            .map_err(|_| "hot loader event channel closed".to_string())
    }

    /// Register an IPC connection as a subscriber (idempotent per `conn_id`).
    ///
    /// `sender` receives raw JSON notification bytes (no 4-byte frame prefix —
    /// the frame is added by the `handle_connection` writer loop).
    pub fn subscribe_conn(&self, conn_id: u64, sender: mpsc::Sender<Vec<u8>>) {
        self.inner.subscribers.insert(conn_id, sender);
    }

    /// Remove a connection's subscription (called on disconnect).
    pub fn unsubscribe(&self, conn_id: u64) {
        self.inner.subscribers.remove(&conn_id);
    }

    /// Return the list of all watched directory paths.
    pub fn watched_dirs(&self) -> Vec<String> {
        self.inner
            .watched_dirs
            .iter()
            .map(|e| e.key().to_string_lossy().into_owned())
            .collect()
    }
}

// ─── Background dispatch loop ─────────────────────────────────────────────────

/// Reads changed paths from the channel, applies a 50 ms debounce window, then
/// fans out `tool/hot_reloaded` notifications to all live subscribers.
async fn dispatch_loop(inner: Arc<HotLoaderInner>, mut rx: mpsc::Receiver<PathBuf>) {
    while let Some(first) = rx.recv().await {
        // 50 ms debounce: collect all rapid follow-up changes.
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let mut paths = std::collections::HashSet::new();
        paths.insert(first);
        while let Ok(p) = rx.try_recv() {
            paths.insert(p);
        }

        for path in paths {
            send_reload_notification(&inner, &path);
        }
    }
}

/// Build a `tool/hot_reloaded` JSON-RPC notification and push it to all
/// live subscribers. Dead senders (closed connections) are pruned.
fn send_reload_notification(inner: &HotLoaderInner, path: &Path) {
    let msg = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "method": "tool/hot_reloaded",
        "params": {
            "path": path.to_string_lossy(),
        }
    }))
    .unwrap_or_default();

    let dead: Vec<u64> = inner
        .subscribers
        .iter()
        .filter_map(|entry| {
            if entry.value().try_send(msg.clone()).is_err() {
                Some(*entry.key())
            } else {
                None
            }
        })
        .collect();

    for id in dead {
        inner.subscribers.remove(&id);
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_hot_loader_new() {
        let (handle, task) = HotLoaderHandle::new(vec!["lua".to_string()]);
        assert!(handle.watched_dirs().is_empty());
        task.abort();
    }

    #[tokio::test]
    async fn test_watch_dir_creates_missing_dir() {
        let tmp = std::env::temp_dir().join("claw_hot_loader_test_watch_dir");
        let _ = std::fs::remove_dir_all(&tmp);
        let (handle, task) = HotLoaderHandle::new(vec!["lua".to_string()]);
        handle.watch_dir(tmp.clone()).expect("watch_dir should succeed");
        assert!(tmp.exists());
        assert!(handle.watched_dirs().iter().any(|d| d.contains("claw_hot_loader_test_watch_dir")));
        let _ = std::fs::remove_dir_all(&tmp);
        task.abort();
    }

    #[tokio::test]
    async fn test_watch_dir_idempotent() {
        let tmp = std::env::temp_dir().join("claw_hot_loader_test_idempotent");
        let _ = std::fs::remove_dir_all(&tmp);
        let (handle, task) = HotLoaderHandle::new(vec!["lua".to_string()]);
        handle.watch_dir(tmp.clone()).expect("first watch_dir");
        handle.watch_dir(tmp.clone()).expect("second watch_dir should be idempotent");
        let dirs = handle.watched_dirs();
        assert_eq!(dirs.len(), 1);
        let _ = std::fs::remove_dir_all(&tmp);
        task.abort();
    }

    #[tokio::test]
    async fn test_subscribe_and_unsubscribe() {
        let (handle, task) = HotLoaderHandle::new(vec!["lua".to_string()]);
        let (tx, _rx) = mpsc::channel::<Vec<u8>>(8);
        handle.subscribe_conn(42, tx);
        assert!(handle.inner.subscribers.contains_key(&42));
        handle.unsubscribe(42);
        assert!(!handle.inner.subscribers.contains_key(&42));
        task.abort();
    }

    #[tokio::test]
    async fn test_trigger_reload_delivers_notification() {
        let (handle, _task) = HotLoaderHandle::new(vec!["lua".to_string()]);
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(8);
        handle.subscribe_conn(1, tx);

        let path = PathBuf::from("/workspace/tools/my_tool.lua");
        handle.trigger_reload(path).await.expect("trigger_reload should succeed");

        // Give the dispatch loop 200 ms to process.
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        let msg = rx.try_recv().expect("should have received notification");
        let json: serde_json::Value = serde_json::from_slice(&msg).unwrap();
        assert_eq!(json["method"], "tool/hot_reloaded");
        assert!(json["params"]["path"].as_str().unwrap().contains("my_tool.lua"));
    }
}
