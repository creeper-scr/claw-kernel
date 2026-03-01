//! Traits for claw-pal.
//!
//! Provides core trait definitions for sandbox backends and other extensible components.

pub mod sandbox;
pub mod process;
pub mod ipc;

pub use sandbox::SandboxBackend;
pub use process::ProcessManager;
pub use ipc::IpcTransport;
