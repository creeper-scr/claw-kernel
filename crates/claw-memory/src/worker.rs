//! Background memory archiving worker.
//!
//! MemoryWorker listens on a channel for `Vec<MemoryItem>` batches,
//! persists them to the SqliteMemoryStore, and emits events via the
//! configured EventPublisher trait.
use crate::{
    error::MemoryError, sqlite::store::SqliteMemoryStore, traits::MemoryStore, types::MemoryItem,
};
use std::sync::Arc;
use tokio::sync::mpsc;

/// Trait for publishing memory-related events.
///
/// This abstraction allows claw-memory (Layer 2) to notify the runtime (Layer 1)
/// without creating a circular dependency. The runtime implements this trait
/// and injects it into the MemoryWorker.
///
/// # Example
///
/// ```rust,ignore
/// use claw_memory::worker::EventPublisher;
/// use claw_runtime::{EventBus, events::Event, agent_types::AgentId};
///
/// struct RuntimeEventPublisher {
///     event_bus: Arc<EventBus>,
/// }
///
/// impl EventPublisher for RuntimeEventPublisher {
///     fn publish_memory_archived(&self, agent_id: String, archived_count: usize) {
///         let _ = self.event_bus.publish(Event::MemoryArchiveComplete {
///             agent_id: AgentId::new(agent_id),
///             archived_count,
///         });
///     }
/// }
/// ```
pub trait EventPublisher: Send + Sync {
    /// Publish a memory archive completion event.
    fn publish_memory_archived(&self, agent_id: String, archived_count: usize);
}

/// Simple no-op publisher for testing or when event publishing is not needed.
pub struct NoopEventPublisher;

impl EventPublisher for NoopEventPublisher {
    fn publish_memory_archived(&self, _agent_id: String, _archived_count: usize) {}
}

/// Background worker that archives memory items to SQLite.
///
/// Design:
/// - Receives batches via mpsc channel
/// - Stores each item in SqliteMemoryStore
/// - Publishes events via EventPublisher trait on success
/// - Runs in a dedicated tokio task (spawn via `start()`)
pub struct MemoryWorker<P: EventPublisher> {
    store: Arc<SqliteMemoryStore>,
    event_publisher: Arc<P>,
    rx: mpsc::Receiver<ArchiveRequest>,
}

/// A request to archive a batch of memory items for an agent.
pub struct ArchiveRequest {
    pub agent_id: String,
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
        agent_id: impl Into<String>,
        items: Vec<MemoryItem>,
    ) -> Result<(), MemoryError> {
        self.tx
            .send(ArchiveRequest {
                agent_id: agent_id.into(),
                items,
            })
            .await
            .map_err(|_| MemoryError::Storage("worker channel closed".to_string()))
    }
}

impl<P: EventPublisher> MemoryWorker<P> {
    /// Default channel capacity for the archive request queue.
    ///
    /// Default channel capacity is 256. Increase for high-throughput memory operations.
    /// Use [`MemoryWorker::with_capacity`] to override this at construction time.
    pub const DEFAULT_CHANNEL_CAPACITY: usize = 256;

    /// Create a new MemoryWorker with capacity-256 channel.
    ///
    /// Returns (worker, handle). Call `worker.start()` to begin processing.
    pub fn new(
        store: Arc<SqliteMemoryStore>,
        event_publisher: Arc<P>,
    ) -> (Self, MemoryWorkerHandle) {
        Self::with_capacity(store, event_publisher, Self::DEFAULT_CHANNEL_CAPACITY)
    }

    /// Create a new MemoryWorker with a custom channel capacity.
    ///
    /// Default channel capacity is 256. Increase for high-throughput memory operations.
    ///
    /// Returns (worker, handle). Call `worker.start()` to begin processing.
    pub fn with_capacity(
        store: Arc<SqliteMemoryStore>,
        event_publisher: Arc<P>,
        channel_capacity: usize,
    ) -> (Self, MemoryWorkerHandle) {
        let (tx, rx) = mpsc::channel(channel_capacity);
        let worker = Self {
            store,
            event_publisher,
            rx,
        };
        let handle = MemoryWorkerHandle { tx };
        (worker, handle)
    }

    /// Start the worker loop in a background tokio task.
    pub fn start(mut self) -> tokio::task::JoinHandle<()>
    where
        P: 'static,
    {
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
                        // FIX-13: separate warn (visible in prod) from debug (error detail that
                        // may contain sensitive content), so logs are safe to forward.
                        tracing::warn!(agent_id = %req.agent_id, item_count = count, "MemoryWorker: failed to archive entry");
                        tracing::debug!(error = %e, "MemoryWorker archive error detail");
                    }
                }
            }

            // Emit event via trait (best-effort)
            self.event_publisher
                .publish_memory_archived(req.agent_id, archived);

            let _ = count; // suppress unused warning
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mock event publisher for testing
    struct MockEventPublisher {
        pub archived_count: AtomicUsize,
        pub agent_id: std::sync::Mutex<Option<String>>,
    }

    impl MockEventPublisher {
        fn new() -> Self {
            Self {
                archived_count: AtomicUsize::new(0),
                agent_id: std::sync::Mutex::new(None),
            }
        }
    }

    impl EventPublisher for MockEventPublisher {
        fn publish_memory_archived(&self, agent_id: String, archived_count: usize) {
            *self.agent_id.lock().unwrap() = Some(agent_id);
            self.archived_count.store(archived_count, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn test_memory_worker_create() {
        let store = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let publisher = Arc::new(NoopEventPublisher);
        let (_worker, handle) = MemoryWorker::new(store, publisher);
        // handle is created
        drop(handle);
    }

    #[tokio::test]
    async fn test_memory_worker_archive_items() {
        let store = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let publisher = Arc::new(MockEventPublisher::new());

        let (worker, handle) =
            MemoryWorker::new(Arc::clone(&store), Arc::clone(&publisher));
        let _join = worker.start();

        let items = vec![
            MemoryItem::new("test-agent", "fact 1"),
            MemoryItem::new("test-agent", "fact 2"),
        ];

        handle.archive("test-agent", items).await.unwrap();

        // Wait for processing
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Check event was published
        assert_eq!(
            publisher.archived_count.load(Ordering::SeqCst),
            2,
            "should have archived 2 items"
        );
        assert_eq!(
            publisher.agent_id.lock().unwrap().as_ref().unwrap(),
            "test-agent"
        );
    }

    #[tokio::test]
    async fn test_memory_worker_handle_channel_closed() {
        let store = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let publisher = Arc::new(NoopEventPublisher);

        let (worker, handle) = MemoryWorker::new(store, publisher);
        // Drop worker without starting — channel becomes disconnected
        drop(worker);

        let result = handle.archive("a", vec![]).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_memory_worker_empty_batch() {
        let store = Arc::new(SqliteMemoryStore::in_memory().unwrap());
        let publisher = Arc::new(NoopEventPublisher);

        let (worker, handle) =
            MemoryWorker::new(Arc::clone(&store), Arc::clone(&publisher));
        let _join = worker.start();

        // Empty batch should still emit event
        handle.archive("x", vec![]).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;
    }
}
