//! Platform Abstraction Layer for claw-kernel.
//!
//! This crate provides cross-platform abstractions for:
//! - IPC (Inter-Process Communication)
//! - Process management
//! - Sandbox/security backends
//!
//! # Main Types
//!
//! - [`IpcTransport`] - Trait for IPC implementations
//! - [`InterprocessTransport`] - Unix socket/named pipe transport
//! - [`ProcessManager`] - Trait for process management
//! - [`TokioProcessManager`] - Tokio-based process manager
//! - [`SandboxBackend`] - Trait for sandbox implementations
//! - [`SandboxConfig`] - Configuration for sandbox policies
//!
//! # Example
//!
//! ```rust,ignore
//! use claw_pal::{InterprocessTransport, TokioProcessManager};
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Create IPC transport as client (connects to an existing server)
//! let transport = InterprocessTransport::new_client("/tmp/claw.sock").await?;
//!
//! // Or create as server (binds and accepts one connection)
//! // let transport = InterprocessTransport::new_server("/tmp/claw.sock").await?;
//!
//! // Create process manager
//! let process_mgr = TokioProcessManager::new();
//!
//! // Spawn a managed process
//! // let handle = process_mgr.spawn(config).await?;
//! # Ok(())
//! # }
//! ```

pub mod config;
pub mod dirs;
pub mod error;
pub mod manager;
pub mod security;
pub mod traits;

// Internal modules - implementation details hidden from public API
pub(crate) mod ipc;
pub(crate) mod types;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
mod windows;

// Error types
pub use error::{IpcError, PalError, ProcessError, SandboxError};

// Core traits
pub use traits::{IpcTransport, ProcessManager, SandboxBackend};

// IPC implementation and types
pub use ipc::InterprocessTransport;
pub use traits::ipc::MockIpcTransport;
pub use types::ipc::{IpcConnection, IpcEndpoint, IpcListener, IpcMessage};

// Process types
pub use types::process::{ExitStatus, ProcessConfig, ProcessHandle, ProcessSignal};

// Sandbox types (from traits::sandbox)
pub use traits::sandbox::{
    ExecutionMode, PlatformHandle, SandboxConfig, SandboxHandle, SyscallPolicy,
};

// Policy types
pub use types::{NetRule, PathRule, ResourceLimits};

// Process manager implementation
pub use manager::TokioProcessManager;

// Security types
pub use security::{ModeTransitionGuard, PowerKey, PowerKeyHash, PowerKeyManager, PowerKeyValidator, SecurityError};
