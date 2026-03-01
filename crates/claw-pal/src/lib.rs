//! Platform Abstraction Layer for claw-kernel.

pub mod dirs;
pub mod error;
pub mod traits;
pub mod types;
pub mod security;

pub use error::{IpcError, PalError, ProcessError, SandboxError};
pub use traits::{ProcessManager, SandboxBackend};
pub use types::{NetRule, PathRule, ResourceLimits};
pub use types::process::{ExitStatus, ProcessConfig, ProcessHandle, ProcessSignal};
