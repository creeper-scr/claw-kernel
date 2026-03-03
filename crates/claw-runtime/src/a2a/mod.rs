//! A2A (Agent-to-Agent) Communication Module
//!
//! This module provides the protocol definitions and routing infrastructure
//! for inter-agent communication in the claw-kernel runtime.
//!
//! ## Module Structure
//!
//! - `protocol`: Core A2A protocol types (messages, priorities, capabilities)
//! - `routing`: Simple message routing

pub mod protocol;
pub mod routing;

// Re-export commonly used types from protocol
pub use protocol::{
    A2AMessage, A2AMessagePayload, A2AMessageType, AgentCapability, MessagePriority,
    ResponseStatus, TaskSpec,
};
// Re-export types from routing
pub use routing::{AgentHandle, SimpleRouter};
