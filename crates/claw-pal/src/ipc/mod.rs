//! IPC transport implementation for claw-pal.
//!
//! Provides length-prefixed framing and cross-platform socket transport
//! backed by the `interprocess` crate.

pub mod framing;
pub mod transport;

pub use transport::InterprocessTransport;
