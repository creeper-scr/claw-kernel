//! System runtime — EventBus, IpcRouter, AgentOrchestrator, A2A Protocol,
//! Schedule, Webhook.
//!
//! This crate provides the runtime infrastructure for managing agents,
//! including event distribution, IPC routing, process management,
//! agent-to-agent (A2A) communication, scheduled tasks, and webhook endpoints.
//!
//! # Main Types
//!
//! - [`Runtime`] - Main runtime for managing the system
//! - [`EventBus`] - Broadcast channel for events
//! - [`AgentOrchestrator`] - Manages agent lifecycle
//! - [`IpcRouter`] - Routes messages between processes
//! - [`A2AMessage`] - Agent-to-agent message protocol
//! - [`AgentId`], [`AgentHandle`] - Agent identification and control
//! - [`Scheduler`] - Time-triggered task scheduling
//! - [`WebhookServer`] - External HTTP event input
//!
//! # Example
//!
//! ```rust,ignore
//! use claw_runtime::{Runtime, EventBus, AgentOrchestrator};
//! use claw_runtime::agent_types::{AgentConfig, ExecutionMode};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create the runtime
//! let runtime = Runtime::new();
//!
//! // Get the event bus for subscribing to events
//! let event_bus = runtime.event_bus();
//!
//! // Create an orchestrator for managing agents
//! let orchestrator = AgentOrchestrator::new();
//!
//! // Register an agent
//! let config = AgentConfig {
//!     name: "my-agent".to_string(),
//!     execution_mode: ExecutionMode::Safe,
//!     // ... other config
//! };
//! // orchestrator.spawn(config).await?;
//! # Ok(())
//! # }
//! ```

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
pub mod schedule;

#[cfg(feature = "webhook")]
pub mod webhook;

pub use a2a::{
    A2AMessage, A2AMessagePayload, A2AMessageType, AgentCapability, MessagePriority,
    ResponseStatus, SimpleRouter, TaskSpec,
};
pub use agent_types::{AgentConfig, AgentHandle, AgentId, AgentInfo, AgentStatus};
// Re-export PAL types needed by callers of AgentOrchestrator::spawn.
pub use claw_pal::{ExecutionMode, ExitStatus, ProcessConfig, ProcessHandle, TokioProcessManager};
// Re-export PAL dirs module for script engines and other consumers.
pub use claw_pal::dirs;
pub use discovery::{AgentCapability as DiscoveryAgentCapability, AgentDiscovery, AgentMeta};
pub use error::RuntimeError;
pub use event_bus::{EventBus, EventFilter, EventReceiver, FilteredReceiver, LagStrategy};
pub use events::Event;
pub use extension::{ExtensionEvent, LoadError, ReloadError};
pub use ipc_router::IpcRouter;
pub use orchestrator::AgentOrchestrator;
pub use process::ManagedProcess;
pub use runtime::Runtime;

// Schedule module exports
pub use schedule::{
    ScheduleError, Scheduler, SchedulerExt, TaskConfig, TaskHandler, TaskId, TaskStats,
    TaskTrigger, TokioScheduler,
};

#[cfg(feature = "webhook")]
pub use webhook::{
    EndpointConfig, EndpointId, HmacConfig, HttpMethod, WebhookConfig, WebhookError,
    WebhookHandler, WebhookRequest, WebhookResponse, WebhookServer, WebhookServerExt,
    WebhookStats, WebhookVerifier,
};

#[cfg(feature = "webhook")]
pub use webhook::verification::{verify_hmac_sha256, HmacSha256Verifier, NoopVerifier};

#[cfg(feature = "webhook")]
pub use webhook::AxumWebhookServer;
