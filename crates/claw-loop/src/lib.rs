//! Agent loop engine, history management, and stop conditions.
//!
//! This crate provides the core agent execution loop that coordinates
//! LLM completions, tool execution, and state management.
//!
//! # Main Types
//!
//! - [`AgentLoop`] - The main agent execution loop
//! - [`AgentLoopBuilder`] - Fluent builder for configuring the loop
//! - [`InMemoryHistory`] - In-memory conversation history
//! - [`HistoryManager`] - Trait for custom history implementations
//! - [`StopCondition`] - Trait for defining loop termination conditions
//! - [`MaxTurns`], [`TokenBudget`], [`NoToolCall`] - Built-in stop conditions
//!
//! # Example
//!
//! ```rust,no_run
//! use claw_loop::AgentLoopBuilder;
//! use claw_provider::OllamaProvider;
//! use std::sync::Arc;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let provider = Arc::new(OllamaProvider::from_env()?);
//! let loop_ = AgentLoopBuilder::new()
//!     .with_provider(provider)
//!     .with_max_turns(10)
//!     .with_system_prompt("You are a helpful assistant.")
//!     .build()?;
//! # Ok(())
//! # }
//! ```

pub mod agent_loop;
pub mod builder;
pub mod error;
pub mod history;
pub mod sqlite_history;
pub mod state_machine;
pub mod stop_conditions;
pub mod summarizer;
pub mod traits;
pub mod types;

pub use agent_loop::AgentLoop;
pub use builder::AgentLoopBuilder;
pub use error::AgentError;
pub use history::InMemoryHistory;
pub use sqlite_history::SqliteHistory;
pub use state_machine::{AgentState, StateEvent, StateMachine, TransitionResult};
pub use stop_conditions::{MaxTurns, NoToolCall, TokenBudget};
pub use summarizer::SimpleSummarizer;
pub use traits::{EventPublisher, HistoryManager, NoopEventPublisher, StopCondition, Summarizer};
pub use types::{AgentLoopConfig, AgentResult, FailoverPolicy, FinishReason, LoopState, StreamChunk};
