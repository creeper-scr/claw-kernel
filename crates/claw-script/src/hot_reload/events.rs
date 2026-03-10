//! Event system for script hot-reloading.

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::broadcast;

use crate::error::ScriptError;
use crate::hot_reload::module::ScriptEntry;
use crate::types::EngineType;

/// Events emitted by the hot-reload system.
#[derive(Debug, Clone)]
pub enum ScriptEvent {
    /// A script was loaded for the first time.
    Loaded {
        /// Script entry metadata.
        entry: ScriptEntry,
        /// Source file path.
        path: PathBuf,
    },

    /// A script was reloaded due to file change.
    Reloaded {
        /// Script entry metadata.
        entry: ScriptEntry,
        /// Source file path.
        path: PathBuf,
        /// Previous version number.
        previous_version: u64,
        /// New version number.
        new_version: u64,
    },

    /// A script was unloaded (file deleted).
    Unloaded {
        /// Script name.
        name: String,
        /// Source file path.
        path: PathBuf,
        /// Last known version.
        version: u64,
    },

    /// Script compilation/validation failed.
    Failed {
        /// Source file path.
        path: PathBuf,
        /// Error that occurred.
        error: ScriptError,
        /// Whether this was a reload attempt.
        was_reload: bool,
    },

    /// A batch of debounced events.
    Debounced {
        /// Number of events coalesced.
        count: usize,
        /// Affected paths.
        paths: Vec<PathBuf>,
    },

    /// Hot-reload system started watching.
    Started {
        /// Watched directories.
        directories: Vec<PathBuf>,
    },

    /// Hot-reload system stopped.
    Stopped,

    /// Script was updated in cache.
    CacheUpdated {
        /// Script name.
        name: String,
        /// Engine type.
        engine: EngineType,
        /// Content hash for cache invalidation.
        content_hash: String,
    },
}

/// Event bus for script hot-reload events.
#[derive(Debug)]
pub struct ScriptEventBus {
    sender: broadcast::Sender<ScriptEvent>,
}

impl ScriptEventBus {
    /// Create a new event bus with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    /// Subscribe to events.
    pub fn subscribe(&self) -> broadcast::Receiver<ScriptEvent> {
        self.sender.subscribe()
    }

    /// Emit an event. Returns the number of active subscribers that received it,
    /// or `0` if there are no subscribers (event is dropped).
    pub fn emit(&self, event: ScriptEvent) -> usize {
        self.sender.send(event).unwrap_or(0)
    }

    /// Get the number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.sender.receiver_count()
    }

    /// Create a sender handle that can be cloned.
    pub fn sender(&self) -> broadcast::Sender<ScriptEvent> {
        self.sender.clone()
    }
}

impl Default for ScriptEventBus {
    fn default() -> Self {
        Self::new(128)
    }
}

/// A filtered event subscriber.
pub struct EventFilter {
    receiver: broadcast::Receiver<ScriptEvent>,
    filter: Arc<dyn Fn(&ScriptEvent) -> bool + Send + Sync>,
}

impl EventFilter {
    /// Create a new filtered subscriber.
    pub fn new<F>(bus: &ScriptEventBus, filter: F) -> Self
    where
        F: Fn(&ScriptEvent) -> bool + Send + Sync + 'static,
    {
        Self {
            receiver: bus.subscribe(),
            filter: Arc::new(filter),
        }
    }

    /// Filter to only successful loads/reloads.
    pub fn success_only(bus: &ScriptEventBus) -> Self {
        Self::new(bus, |e| {
            matches!(
                e,
                ScriptEvent::Loaded { .. } | ScriptEvent::Reloaded { .. }
            )
        })
    }

    /// Filter to only error events.
    pub fn errors_only(bus: &ScriptEventBus) -> Self {
        Self::new(bus, |e| matches!(e, ScriptEvent::Failed { .. }))
    }

    /// Filter to specific script name.
    pub fn for_script(bus: &ScriptEventBus, name: impl Into<String>) -> Self {
        let name = name.into();
        Self::new(bus, move |e| {
            let event_name = match e {
                ScriptEvent::Loaded { entry, .. } => &entry.name,
                ScriptEvent::Reloaded { entry, .. } => &entry.name,
                ScriptEvent::Unloaded { name, .. } => name,
                ScriptEvent::Failed { .. } => return true, // Always include errors
                ScriptEvent::CacheUpdated { name, .. } => name,
                _ => return true,
            };
            event_name == &name
        })
    }

    /// Receive the next matching event.
    pub async fn recv(&mut self) -> Option<ScriptEvent> {
        loop {
            match self.receiver.recv().await {
                Ok(event) if (self.filter)(&event) => return Some(event),
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }

    /// Try to receive without blocking.
    pub fn try_recv(&mut self) -> Result<ScriptEvent, broadcast::error::TryRecvError> {
        loop {
            match self.receiver.try_recv() {
                Ok(event) if (self.filter)(&event) => return Ok(event),
                Ok(_) => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_bus_creation() {
        let bus = ScriptEventBus::new(10);
        assert_eq!(bus.subscriber_count(), 0);
    }

    #[test]
    fn test_event_subscription() {
        let bus = ScriptEventBus::new(10);
        let _rx1 = bus.subscribe();
        let _rx2 = bus.subscribe();
        assert_eq!(bus.subscriber_count(), 2);
    }

    #[test]
    fn test_event_emission() {
        let bus = ScriptEventBus::new(10);
        let mut rx = bus.subscribe();

        let event = ScriptEvent::Stopped;
        bus.emit(event.clone());

        let received = rx.try_recv().unwrap();
        assert!(matches!(received, ScriptEvent::Stopped));
    }

    #[tokio::test]
    async fn test_filtered_receiver() {
        let bus = ScriptEventBus::new(10);
        let mut filtered = EventFilter::success_only(&bus);

        // Emit a successful event
        bus.emit(ScriptEvent::Loaded {
            entry: ScriptEntry {
                name: "test".to_string(),
                engine: EngineType::Lua,
                source: "return 1".to_string(),
                path: PathBuf::from("test.lua"),
                version: 1,
                loaded_at: std::time::SystemTime::now(),
            },
            path: PathBuf::from("test.lua"),
        });

        // Should receive it
        let received = tokio::time::timeout(std::time::Duration::from_millis(100), filtered.recv())
            .await
            .unwrap();
        assert!(received.is_some());
    }

    #[test]
    fn test_default_bus() {
        let bus = ScriptEventBus::default();
        assert!(bus.subscriber_count() == 0);
    }
}
