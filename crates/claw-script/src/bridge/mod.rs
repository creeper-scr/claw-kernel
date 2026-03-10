//! Bridges expose host capabilities to script engines.

pub mod agent;
pub(crate) mod conversion;
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

/// Bridge exposing kernel capabilities to scripts.
///
/// This is an aggregate structure containing all available bridges,
/// providing a unified interface for script engines to access host capabilities.
/// Each bridge is optional, allowing fine-grained control over exposed functionality.
#[derive(Default)]
pub struct RustBridge {
    /// Tools bridge for executing tools from scripts.
    pub tools: Option<ToolsBridge>,
    /// Memory bridge for accessing memory stores.
    pub memory: Option<MemoryBridge>,
    /// Events bridge for pub/sub via EventBus.
    pub events: Option<EventsBridge>,
    /// Filesystem bridge for sandboxed file access.
    pub fs: Option<FsBridge>,
    /// Agent bridge for spawning child agents.
    pub agent: Option<AgentBridge>,
    /// Dirs bridge for accessing system directories.
    pub dirs: Option<DirsBridge>,
}

impl RustBridge {
    /// Create a new empty RustBridge.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add tools bridge.
    pub fn with_tools(mut self, tools: ToolsBridge) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Add memory bridge.
    pub fn with_memory(mut self, memory: MemoryBridge) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Add events bridge.
    pub fn with_events(mut self, events: EventsBridge) -> Self {
        self.events = Some(events);
        self
    }

    /// Add filesystem bridge.
    pub fn with_fs(mut self, fs: FsBridge) -> Self {
        self.fs = Some(fs);
        self
    }

    /// Add agent bridge.
    pub fn with_agent(mut self, agent: AgentBridge) -> Self {
        self.agent = Some(agent);
        self
    }

    /// Add dirs bridge.
    pub fn with_dirs(mut self, dirs: DirsBridge) -> Self {
        self.dirs = Some(dirs);
        self
    }
}
