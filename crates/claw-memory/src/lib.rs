//! Memory system — episodic, semantic, and working memory.

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
pub use worker::{ArchiveRequest, MemoryWorker, MemoryWorkerHandle};
