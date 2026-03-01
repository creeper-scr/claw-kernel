//! Windows platform implementations.
//!
//! Provides Windows-specific sandbox using AppContainer (stub implementation).
//!
//! # Components
//!
//! - [`WindowsSandbox`]: Implements [`SandboxBackend`](crate::traits::SandboxBackend) using:
//!   - **AppContainer API** for process-level sandboxing (stub)
//!   - **Job Objects** for resource limits (stub)
//!
//! # Safety
//!
//! AppContainer restrictions are applied at process creation time and cannot be
//! modified after the process starts. Power mode skips sandbox application entirely.

mod sandbox;

pub use sandbox::WindowsSandbox;
