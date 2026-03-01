//! Traits for claw-pal.
//!
//! Provides core trait definitions for sandbox backends and other extensible components.

pub mod ipc;
pub mod process;
pub mod sandbox;

pub use ipc::IpcTransport;
pub use process::ProcessManager;
pub use sandbox::SandboxBackend;
