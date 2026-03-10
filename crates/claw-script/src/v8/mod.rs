//! V8/TypeScript script engine backed by deno_core.
//!
//! This module provides a JavaScript/TypeScript execution environment
//! with strong sandboxing via V8 isolates.
//!
//! # Features
//!
//! - Full ES2022 support
//! - TypeScript transpilation (via deno_core)
//! - Strong sandboxing (V8 isolate)
//! - Async/await support
//! - Same Bridge API as Lua engine

mod bridge;
mod engine;
mod transpile;

pub use bridge::register_bridges;
pub use engine::{V8Engine, V8EngineOptions};
pub use transpile::{
    is_likely_typescript, transpile_typescript, EcmaScriptTarget, TranspilerOptions,
    TypeScriptTranspiler,
};
