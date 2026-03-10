//! Minimal audit interface used by security primitives in `claw-pal`.
//!
//! This module defines a trait ([`AuditSink`]) and a simple event type
//! ([`SecurityAuditEvent`]) so that [`crate::security::PowerModeGuard`] can
//! write audit records without depending on `claw-tools` (which would create a
//! circular dependency).
//!
//! ## Unification bridge
//!
//! To consolidate both audit systems into a single stream, upper layers
//! (e.g. `claw-runtime`) should use [`ChannelAuditSink`]:
//!
//! ```rust,ignore
//! let (sink, receiver) = ChannelAuditSink::new(256);
//! // Pass `sink` (as AuditSinkHandle) to PowerModeGuard::enter(...)
//! // On the runtime side, poll `receiver` and convert SecurityAuditEvent
//! // into AuditEvent::SecurityModeEntered / SecurityModeExited, then push
//! // those into claw_tools::audit::AuditStore.
//! ```
//!
//! Callers that have access to `claw-tools::audit::AuditLogWriterHandle` should
//! wrap it in [`ChannelAuditSink`], which forwards events as
//! `AuditEvent::ModeSwitch` records.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

// ─── Event type ──────────────────────────────────────────────────────────────

/// A security-layer audit event produced by [`crate::security::PowerModeGuard`].
#[derive(Debug, Clone)]
pub struct SecurityAuditEvent {
    /// Unix timestamp in milliseconds.
    pub timestamp_ms: u64,
    /// Agent that triggered the event.
    pub agent_id: String,
    /// Previous execution mode (e.g. `"safe"`).
    pub from_mode: String,
    /// Next execution mode (e.g. `"power"`).
    pub to_mode: String,
    /// Human-readable reason (e.g. `"power_key_verified"`).
    pub reason: String,
}

impl SecurityAuditEvent {
    /// Create an event stamped with the current wall-clock time.
    pub fn now(agent_id: String, from_mode: &str, to_mode: &str, reason: String) -> Self {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            timestamp_ms,
            agent_id,
            from_mode: from_mode.to_string(),
            to_mode: to_mode.to_string(),
            reason,
        }
    }
}

// ─── Trait ───────────────────────────────────────────────────────────────────

/// Abstraction over an audit log sink used by security primitives.
///
/// Implementations are expected to be cheap to clone (e.g. wrapping an
/// `Arc` or an `mpsc::Sender`).
pub trait AuditSink: Send + Sync {
    /// Write a security audit event.
    ///
    /// This method may be called from inside `Drop`, so it must never block
    /// indefinitely.  Implementations should use a fire-and-forget channel
    /// send and drop the event if the channel is full rather than blocking.
    fn write_security_event(&self, event: SecurityAuditEvent);
}

// ─── Shared wrapper ──────────────────────────────────────────────────────────

/// A cheaply-cloneable, heap-allocated [`AuditSink`] handle.
pub type AuditSinkHandle = Arc<dyn AuditSink>;

// ─── No-op sink ──────────────────────────────────────────────────────────────

/// A no-op [`AuditSink`] that silently discards all events.
///
/// Useful in tests and in contexts where no persistent audit log is configured.
pub struct NoopAuditSink;

impl AuditSink for NoopAuditSink {
    fn write_security_event(&self, _event: SecurityAuditEvent) {}
}

impl NoopAuditSink {
    /// Create a shared no-op sink handle.
    pub fn handle() -> AuditSinkHandle {
        Arc::new(Self)
    }
}

// ─── Channel-based sink (bridge to claw-tools AuditStore) ────────────────────

/// An [`AuditSink`] that forwards events over an `mpsc` channel.
///
/// Pair this with a [`SecurityEventReceiver`] on the `claw-runtime` side.
/// The receiver converts [`SecurityAuditEvent`]s into
/// `claw_tools::audit::AuditEvent::SecurityModeEntered` /
/// `SecurityModeExited` and pushes them into the unified `AuditStore`.
///
/// # Drop safety
///
/// [`write_security_event`] uses a non-blocking `try_send`.  If the channel
/// is full the event is silently dropped rather than blocking the caller
/// (which may be inside `Drop`).
pub struct ChannelAuditSink {
    sender: mpsc::Sender<SecurityAuditEvent>,
}

impl ChannelAuditSink {
    /// Create a new sink / receiver pair.
    ///
    /// `capacity` is the number of events that can be queued before the sink
    /// starts dropping events (fire-and-forget semantics).  A value of 256 is
    /// a reasonable default for interactive workloads.
    #[allow(clippy::new_ret_no_self)]
    pub fn new(capacity: usize) -> (AuditSinkHandle, SecurityEventReceiver) {
        let (tx, rx) = mpsc::channel(capacity);
        let sink = Arc::new(Self { sender: tx });
        let receiver = SecurityEventReceiver { inner: rx };
        (sink, receiver)
    }
}

impl AuditSink for ChannelAuditSink {
    fn write_security_event(&self, event: SecurityAuditEvent) {
        // Fire-and-forget: never block (safe to call from Drop).
        let _ = self.sender.try_send(event);
    }
}

// ─── Receiver (upper-layer bridge) ───────────────────────────────────────────

/// Receives [`SecurityAuditEvent`]s forwarded by a [`ChannelAuditSink`].
///
/// Upper layers (e.g. `claw-runtime`) should poll this receiver and convert
/// each event into the unified `claw_tools::audit::AuditEvent` representation
/// before pushing it into an `AuditStore` / `AuditLogWriterHandle`.
pub struct SecurityEventReceiver {
    inner: mpsc::Receiver<SecurityAuditEvent>,
}

impl SecurityEventReceiver {
    /// Receive the next event, waiting until one is available.
    ///
    /// Returns `None` when all [`ChannelAuditSink`] handles have been dropped.
    pub async fn recv(&mut self) -> Option<SecurityAuditEvent> {
        self.inner.recv().await
    }

    /// Try to receive an event without blocking.
    ///
    /// Returns `Err` if no event is currently queued.
    pub fn try_recv(&mut self) -> Result<SecurityAuditEvent, mpsc::error::TryRecvError> {
        self.inner.try_recv()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_sink_discards_events() {
        let sink = NoopAuditSink::handle();
        // Should not panic
        sink.write_security_event(SecurityAuditEvent::now(
            "agent-1".to_string(),
            "safe",
            "power",
            "test".to_string(),
        ));
    }

    #[test]
    fn test_channel_sink_try_recv() {
        let (sink, mut receiver) = ChannelAuditSink::new(16);

        // Nothing queued yet
        assert!(receiver.try_recv().is_err());

        // Write an event
        sink.write_security_event(SecurityAuditEvent::now(
            "agent-2".to_string(),
            "safe",
            "power",
            "power_key_verified".to_string(),
        ));

        // Should now be receivable
        let ev = receiver.try_recv().expect("event should be queued");
        assert_eq!(ev.agent_id, "agent-2");
        assert_eq!(ev.from_mode, "safe");
        assert_eq!(ev.to_mode, "power");
        assert_eq!(ev.reason, "power_key_verified");

        // Queue is empty again
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn test_channel_sink_full_drops_events() {
        // Capacity = 1
        let (sink, mut receiver) = ChannelAuditSink::new(1);

        let make_event = || {
            SecurityAuditEvent::now("a".to_string(), "safe", "power", "test".to_string())
        };

        sink.write_security_event(make_event()); // fills the channel
        sink.write_security_event(make_event()); // should be silently dropped

        // Only one event received
        assert!(receiver.try_recv().is_ok());
        assert!(receiver.try_recv().is_err(), "second event should have been dropped");
    }

    #[tokio::test]
    async fn test_channel_sink_async_recv() {
        let (sink, mut receiver) = ChannelAuditSink::new(8);

        sink.write_security_event(SecurityAuditEvent::now(
            "agent-3".to_string(),
            "power",
            "safe",
            "session_ended_after_500ms".to_string(),
        ));

        let ev = receiver.recv().await.expect("should receive event");
        assert_eq!(ev.agent_id, "agent-3");
        assert_eq!(ev.from_mode, "power");
        assert_eq!(ev.to_mode, "safe");
    }

    #[tokio::test]
    async fn test_channel_sink_recv_returns_none_when_sink_dropped() {
        let (sink, mut receiver) = ChannelAuditSink::new(8);
        drop(sink);
        assert!(receiver.recv().await.is_none());
    }
}

