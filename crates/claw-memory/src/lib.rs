//! Memory system — episodic, semantic, and working memory.
//!
//! This crate provides persistent storage for agent memory, supporting
//! both episodic (conversation history) and semantic (vector-based) storage.
//!
//! # Main Types
//!
//! - [`SqliteMemoryStore`] - SQLite-backed memory store
//! - [`MemoryStore`] - Trait for memory storage implementations
//! - [`MemoryItem`] - A single memory item with optional embedding
//! - [`EpisodicEntry`] - Conversation turn entry
//! - [`SecureMemoryStore`] - Wrapper with quota enforcement
//! - [`MemoryWorker`] - Background worker for memory operations
//!
//! # Example
//!
//! ```rust
//! use claw_memory::{SqliteMemoryStore, MemoryStore};
//! use claw_memory::types::MemoryItem;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create an in-memory store (for testing)
//! let store = SqliteMemoryStore::in_memory()?;
//!
//! // Store a memory item
//! let item = MemoryItem::new("agent-1", "Remember this important fact");
//! let id = store.store(item).await?;
//!
//! // Retrieve it later
//! let retrieved = store.retrieve(&id).await?;
//! assert!(retrieved.is_some());
//! assert_eq!(retrieved.unwrap().content, "Remember this important fact");
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod error;
pub mod secure;
pub mod sqlite;
pub mod traits;
pub mod types;
pub mod worker;

pub use config::MemorySecurityConfig;
pub use error::MemoryError;
pub use secure::SecureMemoryStore;
pub use sqlite::{HistoryRow, SqliteHistoryStore, SqliteMemoryStore};
pub use traits::MemoryStore;
pub use types::{EpisodeId, EpisodicEntry, EpisodicFilter, MemoryId, MemoryItem};
pub use worker::{ArchiveRequest, EventPublisher, MemoryWorker, MemoryWorkerHandle, NoopEventPublisher};
