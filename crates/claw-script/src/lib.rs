//! Embedded script engines — Lua (default), with bridges to host tools.
//!
//! This crate provides script execution capabilities for the agent system,
//! with Lua as the default engine. Scripts can access tools through bridges.
//!
//! # Main Types
//!
//! - [`ScriptEngine`] - Trait for script engine implementations
//! - [`LuaEngine`] - Lua script engine (requires `engine-lua` feature)
//! - [`Script`] - A script to be executed
//! - [`ScriptContext`] - Execution context for scripts
//! - [`ScriptValue`] - Value type for script returns
//! - Tools bridges: [`ToolsBridge`], [`FsBridge`], [`NetBridge`], etc.
//!
//! # Features
//!
//! - `engine-lua` - Lua script engine support (enabled by default)
//!
//! # Example
//!
//! ```rust,ignore
//! use claw_script::{ScriptEngine, LuaEngine};
//! use claw_script::types::Script;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create the Lua engine
//! let engine = LuaEngine::new()?;
//!
//! // Execute a script
//! let script = Script::from_code(r#"
//!     -- Lua code here
//!     return "Hello from Lua!"
//! "#);
//! let result = engine.execute(script).await?;
//! # Ok(())
//! # }
//! ```

pub mod bridge;
pub mod error;
pub mod traits;
pub mod types;

#[cfg(feature = "engine-lua")]
pub mod lua;

pub use error::{CompileError, ScriptError};
pub use traits::ScriptEngine;
pub use types::{EngineType, FsBridgeConfig, NetBridgeConfig, Script, ScriptContext, ScriptValue};

#[cfg(feature = "engine-lua")]
pub use lua::LuaEngine;

// Re-export bridge types for configuration
pub use bridge::tools::CallerContext;
pub use bridge::{
    AgentBridge, DirsBridge, EventsBridge, FsBridge, MemoryBridge, NetBridge, ToolsBridge,
};
