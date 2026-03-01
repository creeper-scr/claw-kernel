//! Memory system — episodic, semantic, and working memory.

pub mod config;
pub mod embedding;
pub mod error;
pub mod secure;
pub mod sqlite;
pub mod traits;
pub mod types;
pub mod worker;

pub use config::MemorySecurityConfig;
pub use embedding::NgramEmbedder;
pub use error::MemoryError;
pub use secure::SecureMemoryStore;
pub use sqlite::SqliteMemoryStore;
pub use traits::{Embedder, MemoryStore};
pub use types::{EpisodeId, EpisodicEntry, EpisodicFilter, MemoryId, MemoryItem};
pub use worker::{MemoryWorker, MemoryWorkerHandle, ArchiveRequest};
