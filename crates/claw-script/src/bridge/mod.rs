//! Bridges expose host capabilities to script engines.

pub mod agent;
pub(crate) mod conversion;
pub mod dirs;
pub mod events;
pub mod fs;
pub mod llm;
pub mod net;
pub mod tools;
pub mod tools_bridge;

pub use agent::{register_agent, AgentBridge};
pub use dirs::{register_dirs, DirsBridge};
pub use events::{register_events, EventsBridge};
pub use fs::{register_fs, FsBridge};
pub use llm::{register_llm, LlmBridge};
pub use net::{register_net, HttpResponse, NetBridge};
pub use tools::{register_tools, ToolsBridge};
pub use tools_bridge::ToolsBridge as ToolsBridgeSync;

/// Bridge exposing kernel capabilities to scripts.
///
/// This is an aggregate structure containing all available bridges,
/// providing a unified interface for script engines to access host capabilities.
/// Each bridge is optional, allowing fine-grained control over exposed functionality.
///
/// Note: Memory operations are NOT exposed to scripts per the D1 architectural
/// decision (v1.3.0). Scripts should not directly manipulate long-term memory;
/// that is an application-layer concern using the `claw-memory` crate's Rust API.
#[derive(Default)]
pub struct RustBridge {
    /// Tools bridge for executing tools from scripts.
    pub tools: Option<ToolsBridge>,
    /// Events bridge for pub/sub via EventBus.
    pub events: Option<EventsBridge>,
    /// Filesystem bridge for sandboxed file access.
    pub fs: Option<FsBridge>,
    /// Agent bridge for spawning child agents.
    pub agent: Option<AgentBridge>,
    /// Dirs bridge for accessing system directories.
    pub dirs: Option<DirsBridge>,
    /// LLM bridge for calling LLM providers from scripts.
    pub llm: Option<LlmBridge>,
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

    /// Add LLM bridge.
    pub fn with_llm(mut self, llm: LlmBridge) -> Self {
        self.llm = Some(llm);
        self
    }
}
