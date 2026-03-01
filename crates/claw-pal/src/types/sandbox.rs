//! Sandbox-related types.
//!
//! Re-exports sandbox types from the traits module for convenience.

pub use crate::traits::sandbox::{
    ExecutionMode, PlatformHandle, SandboxConfig, SandboxHandle, SyscallPolicy,
};
