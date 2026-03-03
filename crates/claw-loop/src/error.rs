use thiserror::Error;

use crate::state_machine::{AgentState, StateEvent};

#[derive(Debug, Error, Clone)]
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
