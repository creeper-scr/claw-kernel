use std::sync::Arc;

use crate::{
    error::RuntimeError, event_bus::EventBus, events::Event, ipc_router::IpcRouter,
    orchestrator::AgentOrchestrator,
};

// ─── Runtime ──────────────────────────────────────────────────────────────────

/// The main runtime context combining `EventBus`, `IpcRouter`, and
/// `AgentOrchestrator`.
///
/// `Runtime` is the top-level composition root.  It owns each subsystem via
/// `Arc` so that individual components can be shared cheaply with other tasks
/// or stored in external state.
pub struct Runtime {
    pub event_bus: Arc<EventBus>,
    pub orchestrator: Arc<AgentOrchestrator>,
    pub ipc_router: Arc<IpcRouter>,
}

impl Runtime {
    /// Construct a new `Runtime` with the given IPC endpoint.
    ///
    /// This creates a fresh `EventBus`, wires the `AgentOrchestrator` and
    /// `IpcRouter` to it, then wraps everything in `Arc`s.
    pub fn new(ipc_endpoint: impl Into<String>) -> Self {
        let event_bus = Arc::new(EventBus::new());
        let orchestrator = Arc::new(AgentOrchestrator::new(Arc::clone(&event_bus)));
        let ipc_router = Arc::new(IpcRouter::new(Arc::clone(&event_bus), ipc_endpoint));
        Self {
            event_bus,
            orchestrator,
            ipc_router,
        }
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
}
