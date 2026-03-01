//! macOS platform implementations.
//!
//! Provides macOS-specific sandbox using the `sandbox(7)` system (Seatbelt).
//!
//! # Components
//!
//! - [`MacOSSandbox`]: Implements [`SandboxBackend`](crate::traits::SandboxBackend) using:
//!   - **sandbox_init()** C API for process-level sandboxing
//!   - **Apple Sandbox Profile Language (SBPL)** for policy definition
//!
//! # Safety
//!
//! `sandbox_init()` is irreversible once applied — the calling process remains
//! sandboxed for its entire lifetime. Power mode skips sandbox application entirely.

mod sandbox;

pub use sandbox::MacOSSandbox;
