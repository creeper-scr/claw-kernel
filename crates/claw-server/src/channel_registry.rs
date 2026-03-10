//! ChannelRegistry — thread-safe registry for connected channels.
//!
//! Maintains the set of channels registered with the kernel server and
//! provides a lightweight deduplication cache (60-second TTL) to prevent
//! duplicate message delivery.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use tokio::sync::Mutex;

/// Metadata for a registered channel.
#[derive(Debug, Clone)]
pub struct RegisteredChannel {
    /// Unique channel identifier.
    pub channel_id: String,
    /// Channel type: "webhook" | "stdin" | "discord" | custom.
    pub channel_type: String,
    /// Type-specific configuration supplied at registration.
    pub config: serde_json::Value,
    /// Wall-clock time when this channel was registered.
    pub registered_at: Instant,
}

/// Thread-safe registry of channels registered with the kernel.
pub struct ChannelRegistry {
    channels: DashMap<String, RegisteredChannel>,
    /// Dedup cache: message_id → time first seen (60-second TTL).
    seen_ids: Arc<Mutex<HashMap<String, Instant>>>,
}

impl ChannelRegistry {
    /// Creates a new, empty ChannelRegistry.
    pub fn new() -> Self {
        Self {
            channels: DashMap::new(),
            seen_ids: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Registers a channel. Returns an error string if the channel_id is already taken.
    pub fn register(
        &self,
        channel_type: String,
        channel_id: String,
        config: serde_json::Value,
    ) -> Result<(), String> {
        if self.channels.contains_key(&channel_id) {
            return Err(format!("channel '{}' is already registered", channel_id));
        }
        self.channels.insert(
            channel_id.clone(),
            RegisteredChannel {
                channel_id,
                channel_type,
                config,
                registered_at: Instant::now(),
            },
        );
        Ok(())
    }

    /// Unregisters a channel. Returns `true` if the channel existed.
    pub fn unregister(&self, channel_id: &str) -> bool {
        self.channels.remove(channel_id).is_some()
    }

    /// Returns a snapshot of all registered channels.
    pub fn list(&self) -> Vec<RegisteredChannel> {
        self.channels.iter().map(|e| e.value().clone()).collect()
    }

    /// Checks whether a message_id has been seen within the last 60 seconds.
    /// If not seen, records it and returns `false` (process this message).
    /// If already seen, returns `true` (skip — duplicate).
    pub async fn is_duplicate(&self, message_id: &str) -> bool {
        let mut seen = self.seen_ids.lock().await;
        let now = Instant::now();
        let ttl = Duration::from_secs(60);

        // Evict expired entries.
        seen.retain(|_, t| now.duration_since(*t) < ttl);

        if seen.contains_key(message_id) {
            return true;
        }
        seen.insert(message_id.to_string(), now);
        false
    }
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for ChannelRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelRegistry")
            .field("count", &self.channels.len())
            .finish()
    }
}
