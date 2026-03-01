use crate::error::RuntimeError;
use crate::events::Event;
use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 1024;

// ─── EventBus ─────────────────────────────────────────────────────────────────

/// Broadcast event bus with capacity 1024.
///
/// `EventBus` is cheaply clonable; all clones share the same underlying
/// broadcast channel.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
}

impl EventBus {
    /// Create a new `EventBus` with capacity 1024.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { tx }
    }

    /// Publish an event to all subscribers.
    ///
    /// Returns the number of active receivers that received the event.
    /// Returns `Ok(0)` when there are no subscribers (not an error).
    pub fn publish(&self, event: Event) -> Result<usize, RuntimeError> {
        match self.tx.send(event) {
            Ok(n) => Ok(n),
            // `SendError` means no active receivers — treat as zero deliveries,
            // not a failure. The channel itself is still usable.
            Err(_) => Ok(0),
        }
    }

    /// Subscribe to all events.
    pub fn subscribe(&self) -> EventReceiver {
        EventReceiver {
            rx: self.tx.subscribe(),
        }
    }

    /// Subscribe to events matching a filter predicate.
    ///
    /// The returned `FilteredReceiver` skips events that do not satisfy `filter`.
    pub fn subscribe_filtered<F>(&self, filter: F) -> FilteredReceiver
    where
        F: Fn(&Event) -> bool + Send + Sync + 'static,
    {
        FilteredReceiver {
            rx: self.tx.subscribe(),
            filter: Box::new(filter),
        }
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

// ─── EventReceiver ────────────────────────────────────────────────────────────

/// Wraps a broadcast receiver for consuming events.
pub struct EventReceiver {
    rx: broadcast::Receiver<Event>,
}

impl EventReceiver {
    /// Asynchronously wait for the next event.
    ///
    /// Returns `Err(RuntimeError::EventBusError)` if the channel is closed or
    /// if the receiver has lagged behind (missed events are reported as an error).
    pub async fn recv(&mut self) -> Result<Event, RuntimeError> {
        match self.rx.recv().await {
            Ok(event) => Ok(event),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // Receiver was too slow; some messages were dropped.
                // Continue from the newest available message.
                Err(RuntimeError::EventBusError(format!(
                    "receiver lagged, {} messages dropped",
                    n
                )))
            }
            Err(broadcast::error::RecvError::Closed) => Err(RuntimeError::EventBusError(
                "event bus channel closed".to_string(),
            )),
        }
    }

    /// Try to receive an event without blocking.
    ///
    /// Returns `Err(RuntimeError::EventBusError("empty"))` when no message is
    /// currently available.
    pub fn try_recv(&mut self) -> Result<Event, RuntimeError> {
        match self.rx.try_recv() {
            Ok(event) => Ok(event),
            Err(broadcast::error::TryRecvError::Empty) => {
                Err(RuntimeError::EventBusError("empty".to_string()))
            }
            Err(broadcast::error::TryRecvError::Lagged(n)) => Err(RuntimeError::EventBusError(
                format!("receiver lagged, {} messages dropped", n),
            )),
            Err(broadcast::error::TryRecvError::Closed) => Err(RuntimeError::EventBusError(
                "event bus channel closed".to_string(),
            )),
        }
    }
}

// ─── FilteredReceiver ─────────────────────────────────────────────────────────

/// Filtered event receiver — only yields events matching the predicate.
pub struct FilteredReceiver {
    rx: broadcast::Receiver<Event>,
    filter: Box<dyn Fn(&Event) -> bool + Send + Sync>,
}

impl FilteredReceiver {
    /// Receive the next matching event, skipping non-matching ones.
    ///
    /// Loops internally until a matching event is found or an unrecoverable
    /// channel error occurs.
    pub async fn recv(&mut self) -> Result<Event, RuntimeError> {
        loop {
            match self.rx.recv().await {
                Ok(event) => {
                    if (self.filter)(&event) {
                        return Ok(event);
                    }
                    // Not a match — keep waiting.
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    return Err(RuntimeError::EventBusError(format!(
                        "filtered receiver lagged, {} messages dropped",
                        n
                    )));
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(RuntimeError::EventBusError(
                        "event bus channel closed".to_string(),
                    ));
                }
            }
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_types::AgentId;
    use crate::events::Event;

    // helper: create a simple AgentStarted event
    fn started(id: &str) -> Event {
        Event::AgentStarted {
            agent_id: AgentId::new(id),
        }
    }

    fn stopped(id: &str) -> Event {
        Event::AgentStopped {
            agent_id: AgentId::new(id),
            reason: "test".to_string(),
        }
    }

    // ── 1. test_event_bus_new ────────────────────────────────────────────────
    #[test]
    fn test_event_bus_new() {
        let bus = EventBus::new();
        // Publish with no subscribers should be Ok(0).
        let result = bus.publish(Event::Shutdown);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    // ── 2. test_event_bus_publish_subscribe ─────────────────────────────────
    #[tokio::test]
    async fn test_event_bus_publish_subscribe() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.publish(started("a1")).unwrap();

        let event = rx.recv().await.unwrap();
        matches!(event, Event::AgentStarted { .. });
    }

    // ── 3. test_event_bus_multiple_subscribers ───────────────────────────────
    #[tokio::test]
    async fn test_event_bus_multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        let mut rx3 = bus.subscribe();

        let n = bus.publish(Event::Shutdown).unwrap();
        assert_eq!(n, 3);

        assert!(matches!(rx1.recv().await.unwrap(), Event::Shutdown));
        assert!(matches!(rx2.recv().await.unwrap(), Event::Shutdown));
        assert!(matches!(rx3.recv().await.unwrap(), Event::Shutdown));
    }

    // ── 4. test_event_bus_filtered_receiver_matches ──────────────────────────
    #[tokio::test]
    async fn test_event_bus_filtered_receiver_matches() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe_filtered(|e| matches!(e, Event::Shutdown));

        bus.publish(Event::Shutdown).unwrap();

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, Event::Shutdown));
    }

    // ── 5. test_event_bus_filtered_receiver_skips ────────────────────────────
    #[tokio::test]
    async fn test_event_bus_filtered_receiver_skips() {
        let bus = EventBus::new();
        // Only interested in Shutdown events.
        let mut filtered = bus.subscribe_filtered(|e| matches!(e, Event::Shutdown));
        // Plain subscriber to know when to stop.
        let mut plain = bus.subscribe();

        // Publish two non-matching events, then one matching event.
        bus.publish(started("x")).unwrap();
        bus.publish(stopped("x")).unwrap();
        bus.publish(Event::Shutdown).unwrap();

        // Consume all three from plain to ensure they were sent.
        plain.recv().await.unwrap();
        plain.recv().await.unwrap();
        plain.recv().await.unwrap();

        // Filtered should have skipped the first two and returned Shutdown.
        let event = filtered.recv().await.unwrap();
        assert!(matches!(event, Event::Shutdown));
    }

    // ── 6. test_event_bus_publish_returns_subscriber_count ───────────────────
    #[test]
    fn test_event_bus_publish_returns_subscriber_count() {
        let bus = EventBus::new();
        let _rx1 = bus.subscribe();
        let _rx2 = bus.subscribe();

        let n = bus.publish(Event::Shutdown).unwrap();
        assert_eq!(n, 2);
    }

    // ── 7. test_event_bus_clone ──────────────────────────────────────────────
    #[tokio::test]
    async fn test_event_bus_clone() {
        let bus = EventBus::new();
        let bus2 = bus.clone(); // clone shares the same channel

        let mut rx = bus.subscribe();

        // Publish via the clone — receiver on original should still get it.
        bus2.publish(Event::Shutdown).unwrap();

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, Event::Shutdown));
    }

    // ── 8. test_event_receiver_try_recv_empty ────────────────────────────────
    #[test]
    fn test_event_receiver_try_recv_empty() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        // Nothing published yet → should get an error indicating "empty".
        let result = rx.try_recv();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        // EventBusError("empty") → message contains "empty"
        assert!(
            err_msg.contains("empty"),
            "expected 'empty' in error, got: {}",
            err_msg
        );
    }
}
