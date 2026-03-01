use thiserror::Error;

#[derive(Debug, Error)]
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
}
