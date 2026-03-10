//! Global server-level tool registry.
//!
//! Holds tool definitions registered via `tool.register` IPC method.
//! All agent sessions can access these tools.

use claw_tools::types::PermissionSet;
use dashmap::DashMap;
use tokio::sync::mpsc;

/// Type of tool executor.
#[derive(Debug, Clone)]
pub enum ExecutorType {
    /// External tool: callback is sent to the registering IPC client.
    External,
}

/// A globally registered tool definition.
#[derive(Debug, Clone)]
pub struct GlobalToolDef {
    /// Unique tool name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON Schema for the tool's input parameters.
    pub schema: serde_json::Value,
    /// How this tool is executed.
    pub executor: ExecutorType,
    /// Sender to notify the registering IPC client when tool is called.
    /// Weak semantics: if the connection closes, sends will fail silently.
    pub caller_tx: Option<mpsc::Sender<Vec<u8>>>,
    /// Declared permission set. Kernel stores this for audit logging;
    /// actual enforcement inside the external process is the client's responsibility.
    pub permissions: PermissionSet,
}

/// Thread-safe registry of server-level tool definitions.
pub struct GlobalToolRegistry {
    tools: DashMap<String, GlobalToolDef>,
}

impl GlobalToolRegistry {
    /// Creates a new, empty GlobalToolRegistry.
    pub fn new() -> Self {
        Self {
            tools: DashMap::new(),
        }
    }

    /// Registers a tool. Overwrites if name already exists.
    pub fn register(&self, def: GlobalToolDef) {
        self.tools.insert(def.name.clone(), def);
    }

    /// Unregisters a tool by name. Returns true if it existed.
    pub fn unregister(&self, name: &str) -> bool {
        self.tools.remove(name).is_some()
    }

    /// Returns a snapshot of all registered tools.
    pub fn list(&self) -> Vec<GlobalToolDef> {
        self.tools.iter().map(|e| e.value().clone()).collect()
    }

    /// Gets a single tool by name.
    pub fn get(&self, name: &str) -> Option<GlobalToolDef> {
        self.tools.get(name).map(|e| e.value().clone())
    }

    /// Returns the number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Returns true if no tools are registered.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for GlobalToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for GlobalToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlobalToolRegistry")
            .field("count", &self.tools.len())
            .finish()
    }
}
