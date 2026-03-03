//! Agent loop engine, history management, and stop conditions.

pub mod agent_loop;
pub mod builder;
pub mod error;
pub mod history;
pub mod state_machine;
pub mod stop_conditions;
pub mod summarizer;
pub mod traits;
pub mod types;

pub use agent_loop::AgentLoop;
pub use builder::AgentLoopBuilder;
pub use error::AgentError;
pub use history::InMemoryHistory;
pub use state_machine::{AgentState, StateEvent, StateMachine, TransitionResult};
pub use stop_conditions::{MaxTurns, NoToolCall, TokenBudget};
pub use summarizer::SimpleSummarizer;
pub use traits::{HistoryManager, StopCondition, Summarizer};
pub use types::{AgentLoopConfig, AgentResult, FinishReason, LoopState};
