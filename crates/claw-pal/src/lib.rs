//! Platform Abstraction Layer for claw-kernel.

pub mod dirs;
pub mod error;
pub mod ipc;
pub mod manager;
pub mod security;
pub mod traits;
pub mod types;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

pub use error::{IpcError, PalError, ProcessError, SandboxError};
pub use ipc::InterprocessTransport;
pub use manager::TokioProcessManager;
pub use traits::{IpcTransport, ProcessManager, SandboxBackend};
pub use types::ipc::{IpcConnection, IpcEndpoint, IpcListener, IpcMessage};
pub use types::process::{ExitStatus, ProcessConfig, ProcessHandle, ProcessSignal};
pub use types::{NetRule, PathRule, ResourceLimits};
