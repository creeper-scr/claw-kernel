//! Bridges expose host capabilities to script engines.

pub mod agent;
pub mod dirs;
pub mod events;
pub mod fs;
pub mod memory;
pub mod net;
pub mod tools;
pub mod tools_bridge;

pub use agent::{register_agent, AgentBridge};
pub use dirs::{register_dirs, DirsBridge};
pub use events::{register_events, EventsBridge};
pub use fs::{register_fs, FsBridge};
pub use memory::{register_memory, MemoryBridge};
pub use net::{register_net, HttpResponse, NetBridge};
pub use tools::{register_tools, ToolsBridge};
pub use tools_bridge::ToolsBridge as ToolsBridgeSync;
