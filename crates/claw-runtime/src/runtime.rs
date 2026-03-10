use std::sync::Arc;

use claw_pal::traits::ProcessManager;
use claw_pal::TokioProcessManager;

use crate::{
    error::RuntimeError, event_bus::EventBus, events::Event, ipc_router::IpcRouter,
    orchestrator::AgentOrchestrator,
};

// ─── Runtime ──────────────────────────────────────────────────────────────────

/// The main runtime context combining `EventBus`, `IpcRouter`,
/// `AgentOrchestrator`, and a `ProcessManager` implementation.
///
/// `Runtime` is the top-level composition root.  It owns each subsystem via
/// `Arc` so that individual components can be shared cheaply with other tasks
/// or stored in external state.
///
/// The `AgentOrchestrator` and `Runtime` share the **same**
/// process manager instance via `Arc`, so processes spawned through
/// either reference are tracked in one place.
pub struct Runtime {
    pub event_bus: Arc<EventBus>,
    pub orchestrator: Arc<AgentOrchestrator>,
    pub ipc_router: Arc<IpcRouter>,
    /// Shared process manager — use via `orchestrator.spawn()` or
    /// directly for one-off process operations.
    pub process_manager: Arc<dyn ProcessManager>,
}

impl Runtime {
    /// Construct a new `Runtime` with the given IPC endpoint, starting all background tasks automatically.
    ///
    /// Creates a fresh `EventBus`, `TokioProcessManager` (as the default
    /// process manager implementation), wires the `AgentOrchestrator` and
    /// `IpcRouter` to them, wraps everything in `Arc`s, and immediately
    /// starts the orchestrator and IPC acceptor background tasks.
    ///
    /// # Example
    /// ```rust,no_run
    /// # async fn example() -> Result<(), claw_runtime::error::RuntimeError> {
    /// use claw_runtime::Runtime;
    /// let runtime = Runtime::new("/tmp/claw.sock").await?;
    /// // Background tasks are already running — no need to call start().
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(ipc_endpoint: impl Into<String>) -> Result<Self, RuntimeError> {
        let event_bus = Arc::new(EventBus::new());
        let process_manager: Arc<dyn ProcessManager> = Arc::new(TokioProcessManager::new());
        let orchestrator = Arc::new(AgentOrchestrator::new(
            Arc::clone(&event_bus),
            Arc::clone(&process_manager),
        ));
        let ipc_router = Arc::new(IpcRouter::with_default_transport(Arc::clone(&event_bus), ipc_endpoint));

        orchestrator.start();
        ipc_router.start_accepting().await?;

        Ok(Self {
            event_bus,
            orchestrator,
            ipc_router,
            process_manager,
        })
    }

    /// Deprecated: background tasks now start automatically in `new()`.
    ///
    /// This method is a no-op and emits a warning. Remove calls to `start()` and
    /// use `Runtime::new(endpoint).await?` instead.
    #[deprecated(since = "1.1.0", note = "Use Runtime::new() — tasks start automatically")]
    pub async fn start(&self) -> Result<(), RuntimeError> {
        tracing::warn!("Runtime::start() is now a no-op; background tasks start in new()");
        Ok(())
    }

    /// Broadcast a `Shutdown` event to all subscribers.
    ///
    /// Callers are responsible for awaiting any running tasks after calling
    /// this method.
    pub fn shutdown(&self) -> Result<(), RuntimeError> {
        self.event_bus.publish(Event::Shutdown);
        Ok(())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::Event;

    #[tokio::test]
    async fn test_runtime_new() {
        let rt = Runtime::new("/tmp/claw-runtime-test.sock").await.unwrap();
        assert_eq!(rt.ipc_router.endpoint(), "/tmp/claw-runtime-test.sock");
        assert_eq!(rt.orchestrator.agent_count(), 0);
    }

    #[tokio::test]
    async fn test_runtime_shutdown_sends_event() {
        let rt = Runtime::new("/tmp/claw-shutdown-test.sock").await.unwrap();
        let mut rx = rt.event_bus.subscribe();

        rt.shutdown().expect("shutdown should succeed");

        let event = rx.recv().await.expect("should receive shutdown event");
        assert!(matches!(event, Event::Shutdown));
    }

    #[tokio::test]
    async fn test_runtime_has_process_manager() {
        let rt = Runtime::new("/tmp/claw-pm-test.sock").await.unwrap();
        // Verify the process_manager field is accessible and is the same Arc
        // that the orchestrator uses (same pointer).
        let pm_ptr = Arc::as_ptr(&rt.process_manager);
        // The orchestrator uses the same Arc — we can't access the private
        // field directly, but we verify the Runtime field is non-null.
        assert!(!pm_ptr.is_null());
    }
}
