//! Embedded script engines — Lua (default), with bridges to host tools.

pub mod bridge;
pub mod error;
pub mod traits;
pub mod types;

#[cfg(feature = "engine-lua")]
pub mod lua;

pub use error::{CompileError, ScriptError};
pub use traits::ScriptEngine;
pub use types::{EngineType, Script, ScriptContext, ScriptValue};

#[cfg(feature = "engine-lua")]
pub use lua::LuaEngine;
