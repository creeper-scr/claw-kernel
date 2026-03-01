//! Linux platform implementations.
//!
//! Provides Linux-specific sandbox using seccomp-bpf and setrlimit.
//!
//! # Components
//!
//! - [`LinuxSandbox`]: Implements [`SandboxBackend`](crate::traits::SandboxBackend) using:
//!   - **seccomp-bpf** for syscall filtering (via libseccomp)
//!   - **setrlimit** for resource limits (via nix crate)
//!
//! # Safety
//!
//! All denied syscalls use `SCMP_ACT_ERRNO(EPERM)` instead of `SCMP_ACT_KILL`
//! to prevent Rust panics when thread joins detect killed threads.

mod sandbox;

pub use sandbox::LinuxSandbox;
