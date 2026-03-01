//! System runtime — EventBus, IpcRouter, AgentOrchestrator.

pub mod agent_types;
pub mod error;
pub mod event_bus;
pub mod events;
pub mod ipc_router;
pub mod orchestrator;
pub mod runtime;

pub use agent_types::{A2AMessage, AgentConfig, AgentHandle, AgentId, AgentInfo, ExecutionMode};
pub use error::RuntimeError;
pub use event_bus::{EventBus, EventReceiver, FilteredReceiver};
pub use events::Event;
pub use ipc_router::IpcRouter;
pub use orchestrator::AgentOrchestrator;
pub use runtime::Runtime;
