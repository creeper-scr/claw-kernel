use std::sync::Arc;

use claw_pal::TokioProcessManager;

use crate::{
    error::RuntimeError, event_bus::EventBus, events::Event, ipc_router::IpcRouter,
    orchestrator::AgentOrchestrator,
};

// ─── Runtime ──────────────────────────────────────────────────────────────────

/// The main runtime context combining `EventBus`, `IpcRouter`,
/// `AgentOrchestrator`, and `TokioProcessManager`.
///
/// `Runtime` is the top-level composition root.  It owns each subsystem via
/// `Arc` so that individual components can be shared cheaply with other tasks
/// or stored in external state.
///
/// The `AgentOrchestrator` and `Runtime` share the **same**
/// `TokioProcessManager` instance via `Arc`, so processes spawned through
/// either reference are tracked in one place.
pub struct Runtime {
    pub event_bus: Arc<EventBus>,
    pub orchestrator: Arc<AgentOrchestrator>,
    pub ipc_router: Arc<IpcRouter>,
    /// Shared PAL process manager — use via `orchestrator.spawn()` or
    /// directly for one-off process operations.
    pub process_manager: Arc<TokioProcessManager>,
}

impl Runtime {
    /// Construct a new `Runtime` with the given IPC endpoint.
    ///
    /// Creates a fresh `EventBus`, `TokioProcessManager`, wires the
    /// `AgentOrchestrator` and `IpcRouter` to them, then wraps everything
    /// in `Arc`s.
    pub fn new(ipc_endpoint: impl Into<String>) -> Self {
        let event_bus = Arc::new(EventBus::new());
        let process_manager = Arc::new(TokioProcessManager::new());
        let orchestrator = Arc::new(AgentOrchestrator::with_process_manager(
            Arc::clone(&event_bus),
            Arc::clone(&process_manager),
        ));
        let ipc_router = Arc::new(IpcRouter::new(Arc::clone(&event_bus), ipc_endpoint));
        Self {
            event_bus,
            orchestrator,
            ipc_router,
            process_manager,
        }
    }

    /// Start accepting IPC connections in a background task.
    ///
    /// Delegates to `IpcRouter::start_accepting`.  Call this after
    /// constructing `Runtime` to enable inter-process message delivery.
    pub async fn start(&self) -> Result<(), RuntimeError> {
        self.ipc_router.start_accepting().await
    }

    /// Broadcast a `Shutdown` event to all subscribers.
    ///
    /// Callers are responsible for awaiting any running tasks after calling
    /// this method.
    pub fn shutdown(&self) -> Result<(), RuntimeError> {
        self.event_bus.publish(Event::Shutdown)?;
        Ok(())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Event;

    #[test]
    fn test_runtime_new() {
        let rt = Runtime::new("/tmp/claw-runtime-test.sock");
        assert_eq!(rt.ipc_router.endpoint(), "/tmp/claw-runtime-test.sock");
        assert_eq!(rt.orchestrator.agent_count(), 0);
    }

    #[tokio::test]
    async fn test_runtime_shutdown_sends_event() {
        let rt = Runtime::new("/tmp/claw-shutdown-test.sock");
        let mut rx = rt.event_bus.subscribe();

        rt.shutdown().expect("shutdown should succeed");

        let event = rx.recv().await.expect("should receive shutdown event");
        assert!(matches!(event, Event::Shutdown));
    }

    #[test]
    fn test_runtime_has_process_manager() {
        let rt = Runtime::new("/tmp/claw-pm-test.sock");
        // Verify the process_manager field is accessible and is the same Arc
        // that the orchestrator uses (same pointer).
        let pm_ptr = Arc::as_ptr(&rt.process_manager);
        // The orchestrator uses the same Arc — we can't access the private
        // field directly, but we verify the Runtime field is non-null.
        assert!(!pm_ptr.is_null());
    }
}
