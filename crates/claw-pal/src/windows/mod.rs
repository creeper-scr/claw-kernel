//! Windows platform implementations.
//!
//! Provides Windows-specific sandbox using Job Objects (degraded isolation).
//!
//! # Components
//!
//! - [`WindowsSandbox`]: Implements [`SandboxBackend`](crate::traits::SandboxBackend) using:
//!   - **Job Objects** for resource limits and subprocess blocking (implemented)
//!   - **AppContainer API** for filesystem/network isolation (planned v1.5.0)
//!
//! # Isolation Level
//!
//! Safe mode on Windows provides **partial isolation**:
//! - ✅ Memory limits (via `JobMemoryLimit`)
//! - ✅ Subprocess blocking (via `ActiveProcessLimit = 1`)
//! - ❌ Filesystem restrictions (not enforced — AppContainer pending)
//! - ❌ Network restrictions (not enforced — AppContainer pending)
//!
//! A `tracing::warn!` is emitted when Safe mode is applied so operators are
//! always informed of the reduced isolation guarantees.
//!
//! # Safety
//!
//! Job Object limits are applied to the current process and enforced by the
//! kernel for the process lifetime. Power mode skips sandbox application entirely.

mod sandbox;

pub use sandbox::WindowsSandbox;
