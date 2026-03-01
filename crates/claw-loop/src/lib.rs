//! Agent loop engine, history management, and stop conditions.

pub mod error;
pub mod traits;
pub mod types;

pub use error::AgentError;
pub use traits::{HistoryManager, StopCondition, Summarizer};
pub use types::{AgentLoopConfig, AgentResult, FinishReason, LoopState};
