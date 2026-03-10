//! Embedded script engines — Lua (default), V8 (optional), with bridges to host tools.
//!
//! This crate provides script execution capabilities for the agent system,
//! with Lua as the default engine and optional V8 for JavaScript/TypeScript.
//! Scripts can access tools through bridges.
//!
//! # Main Types
//!
//! - [`ScriptEngine`] - Trait for script engine implementations
//! - [`LuaEngine`] - Lua script engine (requires `engine-lua` feature)
//! - [`V8Engine`] - V8 script engine (requires `engine-v8` feature)
//! - [`Script`] - A script to be executed
//! - [`ScriptContext`] - Execution context for scripts
//! - [`ScriptValue`] - Value type for script returns
//! - Tools bridges: [`ToolsBridge`], [`FsBridge`], [`NetBridge`], etc.
//!
//! # Features
//!
//! - `engine-lua` - Lua script engine support (enabled by default)
//! - `engine-v8` - V8 JavaScript/TypeScript engine support
//!
//! # Example (Lua)
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
//! let script = Script::lua("test", r#"
//!     -- Lua code here
//!     return "Hello from Lua!"
//! "#);
//! let result = engine.execute(&script, &ScriptContext::new("agent-1")).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Example (JavaScript)
//!
//! ```rust,ignore
//! use claw_script::{ScriptEngine, V8Engine};
//! use claw_script::types::Script;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create the V8 engine
//! let engine = V8Engine::new();
//!
//! // Execute a script
//! let script = Script::javascript("test", r#"
//!     // JavaScript code here
//!     return "Hello from JavaScript!";
//! "#);
//! let result = engine.execute(&script, &ScriptContext::new("agent-1")).await?;
//! # Ok(())
//! # }
//! ```

pub mod bridge;
pub mod error;
#[cfg(feature = "hot-reload")]
pub mod hot_reload;
pub mod traits;
pub mod types;

#[cfg(feature = "engine-lua")]
pub mod lua;

#[cfg(feature = "engine-v8")]
pub mod v8;

#[cfg(feature = "engine-lua")]
pub mod tool_compiler;

pub use error::{CompileError, ScriptError};
pub use traits::ScriptEngine;
pub use types::{EngineType, FsBridgeConfig, ModuleHandle, NetBridgeConfig, Script, ScriptContext, ScriptValue};

#[cfg(feature = "engine-lua")]
pub use lua::LuaEngine;

#[cfg(feature = "engine-lua")]
pub use tool_compiler::LuaToolCompiler;

#[cfg(feature = "engine-v8")]
pub use v8::{
    is_likely_typescript, transpile_typescript, EcmaScriptTarget, TranspilerOptions,
    TypeScriptTranspiler, V8Engine, V8EngineOptions,
};

// Re-export bridge types for configuration
pub use bridge::tools::CallerContext;
pub use bridge::{
    AgentBridge, DirsBridge, EventsBridge, FsBridge, NetBridge, RustBridge, ToolsBridge,
};

// Re-export hot-reload types
#[cfg(feature = "hot-reload")]
pub use hot_reload::{
    HotReloadConfig, HotReloadManager, ScriptEntry, ScriptEvent, ScriptEventBus, ScriptModule,
    ScriptRegistry, ScriptWatcher,
};
