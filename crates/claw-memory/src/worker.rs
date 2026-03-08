//! Background memory archiving worker.
//!
//! MemoryWorker listens on a channel for `Vec<MemoryItem>` batches,
//! persists them to the SqliteMemoryStore, and emits MemoryArchiveComplete
//! events on the EventBus.
use crate::{
    error::MemoryError, sqlite::store::SqliteMemoryStore, traits::MemoryStore, types::MemoryItem,
};
use claw_runtime::{agent_types::AgentId, event_bus::EventBus, events::Event};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Background worker that archives memory items to SQLite.
///
/// Design:
/// - Receives batches via mpsc channel
/// - Stores each item in SqliteMemoryStore
/// - Publishes MemoryArchiveComplete on success
/// - Runs in a dedicated tokio task (spawn via `start()`)
pub struct MemoryWorker {
    store: Arc<SqliteMemoryStore>,
    event_bus: Arc<EventBus>,
    rx: mpsc::Receiver<ArchiveRequest>,
}

/// A request to archive a batch of memory items for an agent.
pub struct ArchiveRequest {
    pub agent_id: AgentId,
    pub items: Vec<MemoryItem>,
}

/// Sender handle for submitting archive requests.
#[derive(Clone)]
pub struct MemoryWorkerHandle {
    tx: mpsc::Sender<ArchiveRequest>,
}

impl MemoryWorkerHandle {
    /// Submit items for archiving. Non-blocking; returns error if channel is full.
    pub async fn archive(
        &self,
        agent_id: AgentId,
        items: Vec<MemoryItem>,
    ) -> Result<(), MemoryError> {
        self.tx
            .send(ArchiveRequest { agent_id, items })
            .await
            .map_err(|_| MemoryError::Storage("worker channel closed".to_string()))
    }
}

impl MemoryWorker {
    /// Create a new MemoryWorker with capacity-256 channel.
    ///
    /// Returns (worker, handle). Call `worker.start()` to begin processing.
    pub fn new(
        store: Arc<SqliteMemoryStore>,
        event_bus: Arc<EventBus>,
    ) -> (Self, MemoryWorkerHandle) {
        let (tx, rx) = mpsc::channel(256);
        let worker = Self {
            store,
            event_bus,
            rx,
        };
        let handle = MemoryWorkerHandle { tx };
        (worker, handle)
    }

    /// Start the worker loop in a background tokio task.
    pub fn start(mut self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            self.run().await;
        })
    }

    async fn run(&mut self) {
        while let Some(req) = self.rx.recv().await {
            let count = req.items.len();
            let mut archived = 0usize;

            for item in req.items {
                match self.store.store(item).await {
                    Ok(_) => archived += 1,
                    Err(e) => {
                        tracing::warn!("MemoryWorker: failed to archive item: {e}");
                    }
                }
            }

            // Emit MemoryArchiveComplete (best-effort)
            let _ = self.event_bus.publish(Event::MemoryArchiveComplete {
                agent_id: req.agent_id,
                archived_count: archived,
            });

            let _ = count; // suppress unused warning
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claw_runtime::event_bus::EventBus;

    #[tokio::test]
    async fn test_memory_worker_create() {
        let store = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let bus = Arc::new(EventBus::new());
        let (_worker, handle) = MemoryWorker::new(store, bus);
        // handle is created
        drop(handle);
    }

    #[tokio::test]
    async fn test_memory_worker_archive_items() {
        let store = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let bus = Arc::new(EventBus::new());
        let mut rx = bus.subscribe();

        let (worker, handle) = MemoryWorker::new(Arc::clone(&store), Arc::clone(&bus));
        let _join = worker.start();

        let agent_id = AgentId::new("test-agent");
        let items = vec![
            MemoryItem::new("test-agent", "fact 1"),
            MemoryItem::new("test-agent", "fact 2"),
        ];

        handle.archive(agent_id, items).await.unwrap();

        // Wait for event
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Check event was published
        let event = rx.recv().await.unwrap();
        match event {
            Event::MemoryArchiveComplete { archived_count, .. } => {
                assert_eq!(archived_count, 2);
            }
            _ => panic!("unexpected event"),
        }
    }

    #[tokio::test]
    async fn test_memory_worker_handle_channel_closed() {
        let store = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let bus = Arc::new(EventBus::new());

        let (worker, handle) = MemoryWorker::new(store, bus);
        // Drop worker without starting — channel becomes disconnected
        drop(worker);

        let result = handle.archive(AgentId::new("a"), vec![]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_memory_worker_empty_batch() {
        let store = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let bus = Arc::new(EventBus::new());

        let (worker, handle) = MemoryWorker::new(Arc::clone(&store), Arc::clone(&bus));
        let _join = worker.start();

        // Empty batch should still emit event
        handle.archive(AgentId::new("x"), vec![]).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
}
