//! System runtime — EventBus, IpcRouter, AgentOrchestrator, A2A Protocol.

pub mod a2a;
pub mod agent_types;
pub mod discovery;
pub mod error;
pub mod event_bus;
pub mod events;
pub mod ipc_router;
pub mod orchestrator;
pub mod runtime;

// Re-export commonly used types from a2a module
pub use a2a::{
    A2AMessage, A2AMessagePayload, A2AMessageType, AgentCapability, MessagePriority,
    ResponseStatus, SimpleRouter, TaskSpec,
};
pub use agent_types::{AgentConfig, AgentHandle, AgentId, AgentInfo, ExecutionMode};
pub use discovery::{AgentCapability as DiscoveryAgentCapability, AgentDiscovery, AgentMeta};
pub use error::RuntimeError;
pub use event_bus::{EventBus, EventReceiver, FilteredReceiver, LagStrategy};
pub use events::Event;
pub use ipc_router::IpcRouter;
pub use orchestrator::AgentOrchestrator;
pub use runtime::Runtime;
