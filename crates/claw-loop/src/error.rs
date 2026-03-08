//! Error types for claw-loop.
//!
//! Provides unified error handling for agent loop operations including provider calls,
//! tool execution, state management, and stop conditions.

use thiserror::Error;

use crate::state_machine::{AgentState, StateEvent};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AgentError {
    #[error("provider error: {0}")]
    Provider(String),

    #[error("tool error: {0}")]
    Tool(String),

    #[error("max turns reached ({0})")]
    MaxTurnsReached(u32),

    #[error("token budget exceeded ({used} > {budget})")]
    TokenBudgetExceeded { used: u64, budget: u64 },

    #[error("stopped by condition: {0}")]
    StopCondition(String),

    #[error("context error: {0}")]
    Context(String),

    #[error("serialization error: {0}")]
    Serialization(String),

    /// Invalid state transition attempted.
    #[error(
        "invalid state transition from {from:?} on event {event:?}, allowed events: {allowed:?}"
    )]
    InvalidStateTransition {
        from: AgentState,
        event: StateEvent,
        allowed: Vec<StateEvent>,
    },

    /// State mismatch detected during execution.
    #[error("state mismatch: expected {expected:?}, found {actual:?}")]
    StateMismatch {
        expected: AgentState,
        actual: AgentState,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agent_error_display() {
        let err = AgentError::Provider("timeout".to_string());
        assert_eq!(err.to_string(), "provider error: timeout");

        let err = AgentError::Tool("not found".to_string());
        assert_eq!(err.to_string(), "tool error: not found");

        let err = AgentError::MaxTurnsReached(100);
        assert_eq!(err.to_string(), "max turns reached (100)");

        let err = AgentError::TokenBudgetExceeded { used: 1000, budget: 800 };
        assert_eq!(err.to_string(), "token budget exceeded (1000 > 800)");

        let err = AgentError::StopCondition("user interrupt".to_string());
        assert_eq!(err.to_string(), "stopped by condition: user interrupt");

        let err = AgentError::Context("overflow".to_string());
        assert_eq!(err.to_string(), "context error: overflow");

        let err = AgentError::Serialization("json error".to_string());
        assert_eq!(err.to_string(), "serialization error: json error");
    }

    #[test]
    fn test_agent_error_clone() {
        let err = AgentError::Provider("timeout".to_string());
        let cloned = err.clone();
        assert_eq!(err, cloned);
    }
}
