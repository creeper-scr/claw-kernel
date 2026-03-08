//! System runtime — EventBus, IpcRouter, AgentOrchestrator, A2A Protocol.

pub mod a2a;
pub mod agent_types;
pub mod discovery;
pub mod error;
pub mod event_bus;
pub mod events;
pub mod extension;
pub mod ipc_router;
pub mod orchestrator;
pub mod process;
pub mod runtime;

pub use a2a::{
    A2AMessage, A2AMessagePayload, A2AMessageType, AgentCapability, MessagePriority,
    ResponseStatus, SimpleRouter, TaskSpec,
};
pub use agent_types::{AgentConfig, AgentHandle, AgentId, AgentInfo, AgentStatus, ExecutionMode};
// Re-export PAL types needed by callers of AgentOrchestrator::spawn.
pub use claw_pal::{ExitStatus, ProcessConfig, ProcessHandle, TokioProcessManager};
pub use discovery::{AgentCapability as DiscoveryAgentCapability, AgentDiscovery, AgentMeta};
pub use error::RuntimeError;
pub use event_bus::{EventBus, EventFilter, EventReceiver, FilteredReceiver, LagStrategy};
pub use events::Event;
pub use extension::{ExtensionEvent, LoadError, ReloadError};
pub use ipc_router::IpcRouter;
pub use orchestrator::AgentOrchestrator;
pub use process::ManagedProcess;
pub use runtime::Runtime;
