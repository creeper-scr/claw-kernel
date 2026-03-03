//! Bridges expose host capabilities to script engines.

pub mod fs;
pub mod net;
pub mod tools;
pub mod tools_bridge;

pub use fs::{register_fs, FsBridge};
pub use net::{register_net, HttpResponse, NetBridge};
pub use tools::{register_tools, ToolsBridge};
pub use tools_bridge::ToolsBridge as ToolsBridgeSync;
