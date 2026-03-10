//! StdinChannel — reads lines from stdin and writes to stdout.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use async_trait::async_trait;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

use crate::{
    error::ChannelError,
    traits::{Channel, ChannelEvent, ChannelEventPublisher},
    types::{ChannelId, ChannelMessage, Platform},
};

/// A channel adapter that reads from stdin and writes to stdout.
pub struct StdinChannel {
    id: ChannelId,
    /// Agent identifier forwarded in published channel events.
    agent_id: String,
    tx: mpsc::Sender<ChannelMessage>,
    rx: Mutex<mpsc::Receiver<ChannelMessage>>,
    connected: Arc<AtomicBool>,
    task_handle: Mutex<Option<JoinHandle<()>>>,
    /// Maximum time to wait for the next inbound message.
    /// A value of `Duration::ZERO` means wait indefinitely (original behaviour).
    recv_timeout: std::time::Duration,
    /// Optional EventBus publisher — wires the channel into the runtime event system.
    event_publisher: Option<Arc<dyn ChannelEventPublisher>>,
}

impl StdinChannel {
    /// Create a new StdinChannel with an internal queue capacity of 64.
    pub fn new(id: ChannelId) -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            id,
            agent_id: String::new(),
            tx,
            rx: Mutex::new(rx),
            connected: Arc::new(AtomicBool::new(false)),
            task_handle: Mutex::new(None),
            recv_timeout: std::time::Duration::ZERO,
            event_publisher: None,
        }
    }

    /// Set the maximum time to wait for the next inbound message.
    ///
    /// When set to a non-zero duration, `recv()` returns
    /// `Err(ChannelError::ReceiveFailed("recv timeout"))` if no message
    /// arrives within the deadline.
    ///
    /// A value of `Duration::ZERO` (the default) disables the timeout so
    /// that `recv()` blocks indefinitely, preserving the original behaviour.
    pub fn with_recv_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.recv_timeout = timeout;
        self
    }

    /// Attach an [`ChannelEventPublisher`] to wire this channel into the
    /// runtime EventBus.
    ///
    /// Once set, `send()` publishes [`ChannelEvent::MessageSent`] and
    /// `recv()` publishes [`ChannelEvent::MessageReceived`] on every
    /// call.  `connect()` and `disconnect()` publish
    /// [`ChannelEvent::ConnectionState`].  All publish calls are
    /// best-effort; failures do not affect the primary send/recv result.
    ///
    /// `agent_id` is included in each published event so the runtime can
    /// correlate events back to the owning agent.
    pub fn with_event_publisher(
        mut self,
        agent_id: impl Into<String>,
        publisher: Arc<dyn ChannelEventPublisher>,
    ) -> Self {
        self.agent_id = agent_id.into();
        self.event_publisher = Some(publisher);
        self
    }

    /// Inject a message directly into the queue (test helper).
    #[cfg(test)]
    pub async fn inject(&self, msg: ChannelMessage) {
        let _ = self.tx.send(msg).await;
    }
}

#[async_trait]
impl Channel for StdinChannel {
    fn platform(&self) -> &str {
        "stdin"
    }

    fn channel_id(&self) -> &ChannelId {
        &self.id
    }

    /// Start a background task that reads lines from stdin.
    async fn connect(&self) -> Result<(), ChannelError> {
        self.connected.store(true, Ordering::SeqCst);

        let tx = self.tx.clone();
        let connected = Arc::clone(&self.connected);
        let id = self.id.clone();

        let handle = tokio::spawn(async move {
            let stdin = tokio::io::stdin();
            let mut lines = BufReader::new(stdin).lines();

            while connected.load(Ordering::SeqCst) {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        let msg = ChannelMessage::inbound(id.clone(), Platform::Stdin, line);
                        if tx.send(msg).await.is_err() {
                            // Receiver dropped — stop reading.
                            break;
                        }
                    }
                    // EOF or error — stop the loop.
                    Ok(None) | Err(_) => break,
                }
            }
        });

        *self.task_handle.lock().await = Some(handle);

        if let Some(pub_) = &self.event_publisher {
            let _ = pub_
                .publish(ChannelEvent::ConnectionState {
                    channel: self.id.to_string(),
                    platform: Platform::Stdin,
                    connected: true,
                })
                .await;
        }
        Ok(())
    }

    /// Stop reading from stdin.
    async fn disconnect(&self) -> Result<(), ChannelError> {
        self.connected.store(false, Ordering::SeqCst);
        if let Some(handle) = self.task_handle.lock().await.take() {
            handle.abort();
        }

        if let Some(pub_) = &self.event_publisher {
            let _ = pub_
                .publish(ChannelEvent::ConnectionState {
                    channel: self.id.to_string(),
                    platform: Platform::Stdin,
                    connected: false,
                })
                .await;
        }
        Ok(())
    }

    /// Write `message.content` followed by a newline to stdout.
    async fn send(&self, message: ChannelMessage) -> Result<(), ChannelError> {
        let mut stdout = tokio::io::stdout();
        let line = format!("{}\n", message.content);
        let write_ok = stdout.write_all(line.as_bytes()).await;
        let flush_ok = stdout.flush().await;
        let result = write_ok
            .and(flush_ok)
            .map_err(|e| ChannelError::SendFailed(e.to_string()));

        if let Some(pub_) = &self.event_publisher {
            let _ = pub_
                .publish(ChannelEvent::MessageSent {
                    agent_id: self.agent_id.clone(),
                    channel: self.id.to_string(),
                    platform: Platform::Stdin,
                    success: result.is_ok(),
                })
                .await;
        }
        result
    }

    /// Receive the next message from the internal queue.
    ///
    /// If `recv_timeout` is non-zero, returns `Err(ReceiveFailed)` if no
    /// message arrives within the configured deadline.
    async fn recv(&self) -> Result<ChannelMessage, ChannelError> {
        let recv_fut = async { self.rx.lock().await.recv().await };
        let result = if self.recv_timeout.is_zero() {
            // FIX-17: original behaviour — block indefinitely.
            recv_fut
                .await
                .ok_or_else(|| ChannelError::ReceiveFailed("disconnected".to_string()))
        } else {
            tokio::time::timeout(self.recv_timeout, recv_fut)
                .await
                .map_err(|_| ChannelError::ReceiveFailed("recv timeout".to_string()))?
                .ok_or_else(|| ChannelError::ReceiveFailed("disconnected".to_string()))
        };

        if let (Ok(msg), Some(pub_)) = (&result, &self.event_publisher) {
            let _ = pub_
                .publish(ChannelEvent::MessageReceived {
                    agent_id: self.agent_id.clone(),
                    channel: self.id.to_string(),
                    platform: Platform::Stdin,
                    content_preview: msg.content.chars().take(64).collect(),
                })
                .await;
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use crate::traits::ChannelEvent;

    // ── helper: a publisher that records all emitted events ─────────────────

    struct CapturingPublisher {
        events: Arc<StdMutex<Vec<ChannelEvent>>>,
    }

    impl CapturingPublisher {
        fn new() -> (Arc<dyn ChannelEventPublisher>, Arc<StdMutex<Vec<ChannelEvent>>>) {
            let events = Arc::new(StdMutex::new(Vec::new()));
            let publisher = Arc::new(Self { events: Arc::clone(&events) });
            (publisher, events)
        }
    }

    #[async_trait::async_trait]
    impl ChannelEventPublisher for CapturingPublisher {
        async fn publish(&self, event: ChannelEvent) -> Result<(), ChannelError> {
            self.events.lock().unwrap().push(event);
            Ok(())
        }
    }

    // ── existing tests ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_stdin_channel_new() {
        // Constructor must not panic.
        let _ch = StdinChannel::new(ChannelId::new("test-stdin"));
    }

    #[tokio::test]
    async fn test_stdin_channel_platform() {
        let ch = StdinChannel::new(ChannelId::new("s1"));
        assert_eq!(ch.platform(), "stdin");
    }

    #[tokio::test]
    async fn test_stdin_channel_send_without_connect() {
        // send() should not panic; actual write may or may not succeed in CI.
        let ch = StdinChannel::new(ChannelId::new("s2"));
        let msg = ChannelMessage::outbound(ChannelId::new("s2"), Platform::Stdin, "hello");
        // We only assert it doesn't panic, not that it succeeds.
        let _ = ch.send(msg).await;
    }

    #[tokio::test]
    async fn test_stdin_channel_manual_recv() {
        let ch = StdinChannel::new(ChannelId::new("s3"));
        let injected =
            ChannelMessage::inbound(ChannelId::new("s3"), Platform::Stdin, "injected line");
        ch.inject(injected.clone()).await;

        let received = ch.recv().await.expect("should receive injected message");
        assert_eq!(received.content, "injected line");
        assert_eq!(received.channel_id, ChannelId::new("s3"));
    }

    // ── event publisher tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_recv_publishes_message_received_event() {
        let (publisher, captured) = CapturingPublisher::new();
        let ch = StdinChannel::new(ChannelId::new("ep-recv"))
            .with_event_publisher("agent-42", publisher);

        // Inject a message so recv() can return immediately.
        ch.inject(ChannelMessage::inbound(
            ChannelId::new("ep-recv"),
            Platform::Stdin,
            "hello world",
        ))
        .await;

        ch.recv().await.expect("recv ok");

        let events = captured.lock().unwrap();
        assert_eq!(events.len(), 1, "expected exactly one event");
        match &events[0] {
            ChannelEvent::MessageReceived {
                agent_id,
                channel,
                platform,
                content_preview,
            } => {
                assert_eq!(agent_id, "agent-42");
                assert_eq!(channel, "ep-recv");
                assert_eq!(*platform, Platform::Stdin);
                assert_eq!(content_preview, "hello world");
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_send_publishes_message_sent_event() {
        let (publisher, captured) = CapturingPublisher::new();
        let ch = StdinChannel::new(ChannelId::new("ep-send"))
            .with_event_publisher("agent-7", publisher);

        let msg = ChannelMessage::outbound(ChannelId::new("ep-send"), Platform::Stdin, "hi");
        let _ = ch.send(msg).await; // stdout write may or may not succeed in CI

        let events = captured.lock().unwrap();
        assert_eq!(events.len(), 1, "expected exactly one event");
        match &events[0] {
            ChannelEvent::MessageSent {
                agent_id,
                channel,
                platform,
                success: _,
            } => {
                assert_eq!(agent_id, "agent-7");
                assert_eq!(channel, "ep-send");
                assert_eq!(*platform, Platform::Stdin);
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_connect_disconnect_publish_connection_state() {
        let (publisher, captured) = CapturingPublisher::new();
        let ch = StdinChannel::new(ChannelId::new("ep-conn"))
            .with_event_publisher("agent-1", publisher);

        ch.connect().await.expect("connect ok");
        ch.disconnect().await.expect("disconnect ok");

        let events = captured.lock().unwrap();
        // Should have: ConnectionState(connected=true), ConnectionState(connected=false)
        assert_eq!(events.len(), 2);
        match &events[0] {
            ChannelEvent::ConnectionState { connected, .. } => assert!(connected),
            other => panic!("expected ConnectionState, got {other:?}"),
        }
        match &events[1] {
            ChannelEvent::ConnectionState { connected, .. } => assert!(!connected),
            other => panic!("expected ConnectionState, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_no_event_without_publisher() {
        // Without a publisher, send/recv must still work normally.
        let ch = StdinChannel::new(ChannelId::new("no-pub"))
            .with_recv_timeout(std::time::Duration::from_millis(50));
        ch.inject(ChannelMessage::inbound(
            ChannelId::new("no-pub"),
            Platform::Stdin,
            "msg",
        ))
        .await;
        ch.recv().await.expect("recv without publisher");
    }
}
