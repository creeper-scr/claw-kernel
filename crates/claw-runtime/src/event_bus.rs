use crate::error::RuntimeError;
use crate::events::Event;
use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 1024;

// ─── EventFilter ──────────────────────────────────────────────────────────────

/// A declarative filter for event subscriptions.
///
/// Use with [`EventBus::subscribe_with_filter`] to subscribe to a predefined
/// category of events without writing a custom closure.  For bespoke predicates
/// use [`EventBus::subscribe_filtered`] instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventFilter {
    /// Accept every event (no filtering).
    #[default]
    All,
    /// Agent lifecycle events: `AgentStarted`, `AgentStopped`.
    AgentLifecycle,
    /// Tool-call events: `ToolCalled`, `ToolResult`.
    ToolCalls,
    /// LLM request events: `LlmRequestStarted`, `LlmRequestCompleted`.
    LlmRequests,
    /// Memory-related events: `ContextWindowApproachingLimit`, `MemoryArchiveComplete`.
    MemoryEvents,
    /// Only the `Shutdown` event.
    ShutdownOnly,
    /// Custom function pointer predicate.
    Custom(fn(&Event) -> bool),
}

impl EventFilter {
    /// Returns `true` if `event` passes this filter.
    pub fn matches(&self, event: &Event) -> bool {
        match self {
            EventFilter::All => true,
            EventFilter::AgentLifecycle => {
                matches!(event, Event::AgentStarted { .. } | Event::AgentStopped { .. })
            }
            EventFilter::ToolCalls => {
                matches!(event, Event::ToolCalled { .. } | Event::ToolResult { .. })
            }
            EventFilter::LlmRequests => {
                matches!(
                    event,
                    Event::LlmRequestStarted { .. } | Event::LlmRequestCompleted { .. }
                )
            }
            EventFilter::MemoryEvents => {
                matches!(
                    event,
                    Event::ContextWindowApproachingLimit { .. }
                        | Event::MemoryArchiveComplete { .. }
                )
            }
            EventFilter::ShutdownOnly => matches!(event, Event::Shutdown),
            EventFilter::Custom(f) => f(event),
        }
    }
}

// ─── LagStrategy ──────────────────────────────────────────────────────────────

/// Strategy for handling lagged (dropped) messages in the event bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LagStrategy {
    /// Return an error when lagged (default behavior).
    #[default]
    Error,
    /// Skip lagged messages and continue receiving new messages.
    Skip,
    /// Log a warning and continue receiving new messages.
    Warn,
}

// ─── EventBus ─────────────────────────────────────────────────────────────────

/// Broadcast event bus with capacity 1024.
///
/// `EventBus` is cheaply clonable; all clones share the same underlying
/// broadcast channel.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Event>,
    lag_strategy: LagStrategy,
}

impl EventBus {
    /// Create a new `EventBus` with capacity 1024.
    ///
    /// Uses `LagStrategy::Error` as the default lag strategy for backward compatibility.
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self {
            tx,
            lag_strategy: LagStrategy::Error,
        }
    }

    /// Create a new `EventBus` with a specific lag strategy.
    pub fn with_lag_strategy(lag_strategy: LagStrategy) -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { tx, lag_strategy }
    }

    /// Create a new `EventBus` with custom capacity and lag strategy.
    ///
    /// 用于测试滞后行为（需要小容量来模拟滞后场景）
    pub fn with_capacity_and_strategy(capacity: usize, lag_strategy: LagStrategy) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx, lag_strategy }
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
            lag_strategy: self.lag_strategy,
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
            lag_strategy: self.lag_strategy,
        }
    }

    /// Subscribe to events matching a declarative [`EventFilter`].
    ///
    /// This is more ergonomic than [`EventBus::subscribe_filtered`] when the desired
    /// event category maps to one of the predefined variants.
    pub fn subscribe_with_filter(&self, filter: EventFilter) -> FilteredReceiver {
        FilteredReceiver {
            rx: self.tx.subscribe(),
            filter: Box::new(move |e| filter.matches(e)),
            lag_strategy: self.lag_strategy,
        }
    }

    /// Get the current lag strategy.
    pub fn lag_strategy(&self) -> LagStrategy {
        self.lag_strategy
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
    lag_strategy: LagStrategy,
}

impl EventReceiver {
    /// Create a new `EventReceiver` with a specific lag strategy.
    pub fn with_lag_strategy(rx: broadcast::Receiver<Event>, lag_strategy: LagStrategy) -> Self {
        Self { rx, lag_strategy }
    }

    /// Asynchronously wait for the next event.
    ///
    /// Returns `Err(RuntimeError::EventBusError)` if the channel is closed.
    ///
    /// The behavior when the receiver has lagged behind depends on the configured
    /// `LagStrategy`:
    /// - `LagStrategy::Error`: Returns an error (original behavior).
    /// - `LagStrategy::Skip`: Skips lagged messages and continues.
    /// - `LagStrategy::Warn`: Logs a warning and continues.
    pub async fn recv(&mut self) -> Result<Event, RuntimeError> {
        loop {
            match self.rx.recv().await {
                Ok(event) => return Ok(event),
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    match self.lag_strategy {
                        LagStrategy::Error => {
                            // Receiver was too slow; some messages were dropped.
                            // Return error (original behavior).
                            return Err(RuntimeError::EventBusError(format!(
                                "receiver lagged, {} messages dropped",
                                n
                            )));
                        }
                        LagStrategy::Skip => {
                            tracing::debug!("Skipped {} lagged messages", n);
                            continue; // 继续接收下一条
                        }
                        LagStrategy::Warn => {
                            tracing::warn!("Receiver lagged, skipped {} messages", n);
                            // 继续接收下一条，不返回错误
                            continue;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => {
                    return Err(RuntimeError::EventBusError(
                        "event bus channel closed".to_string(),
                    ))
                }
            }
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
            Err(broadcast::error::TryRecvError::Lagged(n)) => {
                match self.lag_strategy {
                    LagStrategy::Error => Err(RuntimeError::EventBusError(format!(
                        "receiver lagged, {} messages dropped",
                        n
                    ))),
                    LagStrategy::Skip => {
                        // Cannot skip in non-blocking mode, return error
                        Err(RuntimeError::EventBusError(format!(
                            "receiver lagged (skipped {} messages)",
                            n
                        )))
                    }
                    LagStrategy::Warn => {
                        eprintln!(
                            "[EventReceiver] Warning: receiver lagged (skipped {} messages)",
                            n
                        );
                        Err(RuntimeError::EventBusError(format!(
                            "receiver lagged (warned for {} messages)",
                            n
                        )))
                    }
                }
            }
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
    lag_strategy: LagStrategy,
}

/// Error type for non-blocking receive operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    /// No message is currently available.
    Empty,
    /// The channel has been closed.
    Closed,
    /// The receiver lagged behind and messages were skipped.
    Lagged(u64),
}

impl std::fmt::Display for TryRecvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TryRecvError::Empty => write!(f, "empty"),
            TryRecvError::Closed => write!(f, "channel closed"),
            TryRecvError::Lagged(n) => write!(f, "receiver lagged, {} messages dropped", n),
        }
    }
}

impl std::error::Error for TryRecvError {}

impl FilteredReceiver {
    /// 使用指定策略创建 FilteredReceiver
    pub fn with_strategy<F>(
        rx: broadcast::Receiver<Event>,
        filter: Box<F>,
        strategy: LagStrategy,
    ) -> Self
    where
        F: Fn(&Event) -> bool + Send + Sync + 'static,
    {
        Self {
            rx,
            filter,
            lag_strategy: strategy,
        }
    }

    /// Try to receive a matching event without blocking.
    ///
    /// Returns `Err(TryRecvError::Empty)` when no message is currently available.
    /// Returns `Err(TryRecvError::Closed)` when the channel is closed.
    /// Returns `Err(TryRecvError::Lagged(n))` when the receiver has lagged.
    pub fn try_recv(&mut self) -> Result<Event, TryRecvError> {
        loop {
            match self.rx.try_recv() {
                Ok(event) => {
                    if (self.filter)(&event) {
                        return Ok(event);
                    }
                    // Not a match — keep trying (non-blocking).
                    // If no more messages, next iteration will return Empty.
                    continue;
                }
                Err(broadcast::error::TryRecvError::Empty) => {
                    return Err(TryRecvError::Empty);
                }
                Err(broadcast::error::TryRecvError::Closed) => {
                    return Err(TryRecvError::Closed);
                }
                Err(broadcast::error::TryRecvError::Lagged(n)) => {
                    match self.lag_strategy {
                        LagStrategy::Error => {
                            return Err(TryRecvError::Lagged(n));
                        }
                        LagStrategy::Skip => {
                            tracing::debug!(
                                "FilteredReceiver skipped {} lagged messages in try_recv",
                                n
                            );
                            // Continue trying to receive
                            continue;
                        }
                        LagStrategy::Warn => {
                            tracing::warn!(
                                "FilteredReceiver lagged, skipped {} messages in try_recv",
                                n
                            );
                            // Continue trying to receive
                            continue;
                        }
                    }
                }
            }
        }
    }

    /// Receive the next matching event, skipping non-matching ones.
    ///
    /// Loops internally until a matching event is found or an unrecoverable
    /// channel error occurs.
    ///
    /// The behavior when the receiver has lagged behind depends on the configured
    /// `LagStrategy`:
    /// - `LagStrategy::Error`: Returns an error (original behavior).
    /// - `LagStrategy::Skip`: Skips lagged messages and continues.
    /// - `LagStrategy::Warn`: Logs a warning and continues.
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
                    match self.lag_strategy {
                        LagStrategy::Error => {
                            return Err(RuntimeError::EventBusError(format!(
                                "filtered receiver lagged, {} messages dropped",
                                n
                            )));
                        }
                        LagStrategy::Skip => {
                            tracing::debug!("FilteredReceiver skipped {} lagged messages", n);
                            continue; // 继续接收下一条
                        }
                        LagStrategy::Warn => {
                            tracing::warn!("FilteredReceiver lagged, skipped {} messages", n);
                            // 继续接收下一条，不返回错误
                            continue;
                        }
                    }
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

    // ── 9. test_event_bus_with_lag_strategy ──────────────────────────────────
    #[test]
    fn test_event_bus_with_lag_strategy() {
        let bus = EventBus::with_lag_strategy(LagStrategy::Skip);
        assert_eq!(bus.lag_strategy(), LagStrategy::Skip);

        let bus = EventBus::with_lag_strategy(LagStrategy::Warn);
        assert_eq!(bus.lag_strategy(), LagStrategy::Warn);

        let bus = EventBus::with_lag_strategy(LagStrategy::Error);
        assert_eq!(bus.lag_strategy(), LagStrategy::Error);
    }

    // ── 10. test_lag_strategy_default ────────────────────────────────────────
    #[test]
    fn test_lag_strategy_default() {
        assert_eq!(LagStrategy::default(), LagStrategy::Error);
    }

    // ── 11. test_event_bus_new_uses_error_strategy ───────────────────────────
    #[test]
    fn test_event_bus_new_uses_error_strategy() {
        let bus = EventBus::new();
        assert_eq!(bus.lag_strategy(), LagStrategy::Error);
    }

    // ─── LagStrategy 行为测试 (Agent 4: Red Phase) ────────────────────────────

    use tokio::time::{sleep, Duration};

    /// Test: LagStrategy::Skip - 滞后消息被跳过，能继续接收后续消息
    ///
    /// 注意：这是一个设计验证测试（Red Phase）。当前实现返回错误而不是跳过。
    /// 当 Agent 2/3 实现 Skip 策略后，此测试应该通过。
    #[tokio::test]
    async fn test_event_receiver_lag_skip() {
        // 创建一个小容量 EventBus（容量为2）
        let bus = EventBus::with_capacity_and_strategy(2, LagStrategy::Skip);

        // 创建一个订阅者但不立即接收消息
        let mut rx = bus.subscribe();

        // 快速发布超过容量的消息（3条消息，容量为2）
        // 由于订阅者没有接收任何消息，它会滞后
        bus.publish(started("agent-1")).unwrap();
        bus.publish(started("agent-2")).unwrap();
        bus.publish(started("agent-3")).unwrap();

        // 等待一小会儿确保消息处理完成
        sleep(Duration::from_millis(50)).await;

        // 现在尝试接收 - 使用 Skip 策略应该能继续接收最新消息
        // 预期（当前失败）：当前实现返回错误，但 Skip 策略应该返回最新消息
        let result = rx.recv().await;

        // TODO(Agent 2/3): 当 Skip 策略实现后，此行应通过
        // 当前：返回 Err(Lagged)
        // 期望：返回 Ok(AgentStarted("agent-3"))
        assert!(
            result.is_ok(),
            "Skip 策略不应返回错误，而是跳过滞后消息并返回最新消息"
        );

        // 验证收到的是滞后后保留的消息（agent-2，因为容量为2保留最新的2条）
        if let Ok(Event::AgentStarted { agent_id }) = result {
            // 滞后后，接收者应该能接收到保留的消息（agent-2 和 agent-3）
            assert_eq!(
                agent_id.as_str(),
                "agent-2",
                "Skip 策略应能接收滞后后保留的消息"
            );

            // 应该能继续接收下一条消息（agent-3）
            let result2 = rx.recv().await;
            assert!(result2.is_ok(), "Skip 策略应能继续接收后续消息");
            if let Ok(Event::AgentStarted { agent_id: id2 }) = result2 {
                assert_eq!(id2.as_str(), "agent-3", "应能接收 agent-3");
            }
        }
    }

    /// Test: LagStrategy::Warn - 滞后时记录警告并继续
    ///
    /// 注意：这是一个设计验证测试（Red Phase）。当前实现返回错误而不是警告后继续。
    #[tokio::test]
    async fn test_event_receiver_lag_warn() {
        // 创建一个小容量 EventBus
        let bus = EventBus::with_capacity_and_strategy(2, LagStrategy::Warn);

        // 创建一个订阅者但不立即接收消息
        let mut rx = bus.subscribe();

        // 快速发布超过容量的消息
        bus.publish(started("agent-1")).unwrap();
        bus.publish(started("agent-2")).unwrap();
        bus.publish(started("agent-3")).unwrap();

        sleep(Duration::from_millis(50)).await;

        // 慢订阅者开始接收 - 使用 Warn 策略应该记录警告并返回最新消息
        let result = rx.recv().await;

        // TODO(Agent 2/3): 当 Warn 策略实现后，此行应通过
        assert!(result.is_ok(), "Warn 策略不应返回错误，应记录警告后继续");

        // 验证收到的是滞后后保留的消息
        if let Ok(Event::AgentStarted { agent_id }) = result {
            assert_eq!(
                agent_id.as_str(),
                "agent-2",
                "Warn 策略应能接收滞后后保留的消息"
            );
        }
    }

    /// Test: FilteredReceiver 的 lag 行为 - Skip 策略
    ///
    /// 注意：这是一个设计验证测试（Red Phase）。
    #[tokio::test]
    async fn test_filtered_receiver_lag_skip() {
        // 创建一个小容量 EventBus
        let bus = EventBus::with_capacity_and_strategy(2, LagStrategy::Skip);

        // 创建过滤订阅者，只接收 Shutdown 事件
        let mut filtered_rx = bus.subscribe_filtered(|e| matches!(e, Event::Shutdown));

        // 发送多个不匹配的事件，最后发送一个匹配的事件
        // 由于订阅者没有接收任何消息，它会滞后
        bus.publish(started("agent-1")).unwrap(); // 不匹配
        bus.publish(started("agent-2")).unwrap(); // 不匹配
        bus.publish(Event::Shutdown).unwrap(); // 匹配

        sleep(Duration::from_millis(50)).await;

        // 过滤订阅者应该能收到 Shutdown 消息（跳过滞后的非匹配消息）
        let result = filtered_rx.recv().await;

        // TODO(Agent 2/3): 当 Skip 策略在 FilteredReceiver 中实现后，此行应通过
        // 当前：返回 Err(Lagged)
        // 期望：返回 Ok(Shutdown)
        assert!(result.is_ok(), "FilteredReceiver Skip 策略不应返回错误");
        assert!(
            matches!(result.unwrap(), Event::Shutdown),
            "应收到 Shutdown 事件"
        );
    }

    /// Test: FilteredReceiver 的 lag 行为 - Error 策略（默认）
    ///
    /// 验证默认 Error 策略确实返回错误
    #[tokio::test]
    async fn test_filtered_receiver_lag_error() {
        // 创建一个小容量 EventBus（默认 Error 策略）
        let bus = EventBus::with_capacity_and_strategy(2, LagStrategy::Error);

        // 创建过滤订阅者
        let mut filtered_rx = bus.subscribe_filtered(|e| matches!(e, Event::Shutdown));

        // 发送超过容量的消息
        bus.publish(started("agent-1")).unwrap();
        bus.publish(started("agent-2")).unwrap();
        bus.publish(Event::Shutdown).unwrap();

        sleep(Duration::from_millis(50)).await;

        // Error 策略应该返回错误
        let result = filtered_rx.recv().await;
        assert!(result.is_err(), "Error 策略应返回错误");
    }
}
