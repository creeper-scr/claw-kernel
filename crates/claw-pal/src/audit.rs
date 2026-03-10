//! Minimal audit interface used by security primitives in `claw-pal`.
//!
//! This module defines a trait ([`AuditSink`]) and a simple event type
//! ([`SecurityAuditEvent`]) so that [`crate::security::PowerModeGuard`] can
//! write audit records without depending on `claw-tools` (which would create a
//! circular dependency).
//!
//! Callers that have access to `claw-tools::audit::AuditLogWriterHandle` should
//! wrap it in [`ChannelAuditSink`], which forwards events as
//! `AuditEvent::ModeSwitch` records.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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
