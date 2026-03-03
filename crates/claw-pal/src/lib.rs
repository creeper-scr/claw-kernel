//! Platform Abstraction Layer for claw-kernel.

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
pub use security::{
    PowerKey, PowerKeyHash, PowerKeyManager, PowerKeyValidator, SecurityError,
};
