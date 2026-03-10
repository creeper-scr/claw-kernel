//! Hot-reload system for Layer 3 script engine.
//!
//! Provides file watching, debouncing, and atomic hot-swapping capabilities
//! for Lua and V8 scripts at Layer 3 (Extension Foundation).
//!
//! # Architecture
//!
//! ```text
//! File System ──► ScriptWatcher ──► ScriptEvent ──► HotReloadManager ──► ScriptCache
//!                     │                                │
//!                     └─ debounce (50ms)               ├─ compile & validate
//!                                                      ├─ hot-swap
//!                                                      └─ event notify
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use std::sync::Arc;
//! use claw_script::hot_reload::{HotReloadManager, HotReloadConfig, ScriptWatcher};
//! use claw_script::types::EngineType;
//! use claw_script::LuaEngine;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create script engine
//! let engine = Arc::new(LuaEngine::new());
//!
//! // Create hot-reload manager
//! let config = HotReloadConfig::default();
//! let mut manager = HotReloadManager::new(config, engine)?;
//!
//! // Watch a directory
//! manager.watch_directory("./scripts").await?;
//!
//! // Subscribe to events
//! let mut events = manager.subscribe();
//! tokio::spawn(async move {
//!     while let Ok(event) = events.recv().await {
//!         println!("Script event: {:?}", event);
//!     }
//! });
//!
//! // Start the hot-reload loop
//! manager.start().await?;
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod events;
pub mod manager;
pub mod module;
pub mod watcher;

pub use config::HotReloadConfig;
pub use events::{ScriptEvent, ScriptEventBus};
pub use manager::HotReloadManager;
pub use module::{ScriptEntry, ScriptModule, ScriptRegistry};
pub use watcher::{ScriptWatcher, WatchEvent};
