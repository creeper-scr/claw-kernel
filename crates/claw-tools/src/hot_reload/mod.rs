//! Hot-reload system for tool registry.
//!
//! Provides file watching, debouncing, and atomic hot-swapping capabilities.
//!
//! # Architecture
//!
//! ```text
//! File System ──► FileWatcher ──► WatchEvent ──► HotReloadProcessor ──► ToolRegistry
//!                    │                              │
//!                    └─ debounce (50ms)             └─ compile & hot-swap
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use claw_tools::hot_reload::{FileWatcher, HotReloadProcessor};
//! use claw_tools::{ToolRegistry, HotLoadingConfig};
//! use tokio::sync::mpsc;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let registry = Arc::new(ToolRegistry::new());
//! let config = HotLoadingConfig::default();
//!
//! // Create watcher
//! let mut watcher = FileWatcher::new(&config)?;
//!
//! // Channel for events
//! let (tx, rx) = mpsc::channel(32);
//!
//! // Start processor
//! let processor = HotReloadProcessor::new(registry, config);
//! tokio::spawn(async move {
//!     processor.run(rx).await;
//! });
//!
//! // Forward events
//! tokio::spawn(async move {
//!     while let Some(event) = watcher.recv().await {
//!         let _ = tx.send(event).await;
//!     }
//! });
//! # Ok(())
//! # }
//! ```

pub mod processor;
pub mod validation;
pub mod versioned;
pub mod watcher;

pub use processor::{HotReloadBuilder, HotReloadProcessor, ProcessResult, VersionedToolSet};
pub use validation::ToolWatcher;
pub use versioned::{ModuleVersion, VersionedModule};
pub use watcher::{watch_file, FileWatcher, WatchEvent};
