use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use claw_memory::MemoryStore;
use claw_runtime::{AgentOrchestrator, EventBus};
use claw_tools::{registry::ToolRegistry, types::PermissionSet};
use serde::{Deserialize, Serialize};

/// Supported scripting engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineType {
    Lua,
    /// JavaScript via V8 (deno_core).
    #[cfg(feature = "engine-v8")]
    JavaScript,
    /// TypeScript via V8 (deno_core) with transpilation.
    #[cfg(feature = "engine-v8")]
    TypeScript,
}

/// A compiled/loaded script.
#[derive(Debug, Clone)]
pub struct Script {
    pub name: String,
    pub source: String,
    pub engine: EngineType,
}

impl Script {
    /// Create a new Lua script.
    pub fn lua(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            engine: EngineType::Lua,
        }
    }

    /// Create a new JavaScript script.
    #[cfg(feature = "engine-v8")]
    pub fn javascript(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            engine: EngineType::JavaScript,
        }
    }

    /// Create a new TypeScript script.
    #[cfg(feature = "engine-v8")]
    pub fn typescript(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            engine: EngineType::TypeScript,
        }
    }
}

/// Configuration for filesystem bridge.
#[derive(Debug, Clone, Default)]
pub struct FsBridgeConfig {
    /// Allowed read/write paths.
    pub allowed_paths: HashSet<PathBuf>,
    /// Base directory for resolving relative paths.
    pub base_dir: PathBuf,
}

/// Configuration for network bridge.
#[derive(Debug, Clone, Default)]
pub struct NetBridgeConfig {
    /// Allowed domains (e.g., "api.example.com").
    pub allowed_domains: HashSet<String>,
    /// Allowed ports. Empty means standard ports only.
    pub allowed_ports: HashSet<u16>,
    /// Whether loopback addresses are allowed.
    pub allow_loopback: bool,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl NetBridgeConfig {
    /// Create a config that denies all network access.
    pub fn none() -> Self {
        Self::default()
    }

    /// Create a config with specific allowed domains.
    pub fn with_domains(domains: impl IntoIterator<Item = String>) -> Self {
        Self {
            allowed_domains: domains.into_iter().collect(),
            allowed_ports: [80, 443].iter().cloned().collect(),
            allow_loopback: false,
            timeout_secs: 30,
        }
    }
}

/// Execution context passed to scripts.
#[derive(Clone)]
pub struct ScriptContext {
    /// Agent ID executing the script.
    pub agent_id: String,
    /// Script-accessible global variables (JSON values).
    pub globals: std::collections::HashMap<String, serde_json::Value>,
    /// Maximum execution time for a single script (default 30 s).
    pub timeout: Duration,
    /// Filesystem bridge configuration.
    pub fs_config: FsBridgeConfig,
    /// Network bridge configuration.
    pub net_config: NetBridgeConfig,
    /// Tool permissions for the caller.
    pub permissions: PermissionSet,
    /// Tool registry for executing tools from scripts.
    pub tool_registry: Option<Arc<ToolRegistry>>,
    /// Memory store for the memory bridge.
    pub memory_store: Option<Arc<dyn MemoryStore>>,
    /// Event bus for the events bridge.
    pub event_bus: Option<Arc<EventBus>>,
    /// Agent orchestrator for the agent bridge.
    pub orchestrator: Option<Arc<AgentOrchestrator>>,
    /// Resource holders to keep background resources alive during script execution.
    pub(crate) _resource_holders: Vec<Arc<dyn std::any::Any + Send + Sync>>,
}

impl std::fmt::Debug for ScriptContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptContext")
            .field("agent_id", &self.agent_id)
            .field("globals", &self.globals)
            .field("timeout", &self.timeout)
            .field("fs_config", &self.fs_config)
            .field("net_config", &self.net_config)
            .field("permissions", &self.permissions)
            .field("tool_registry", &self.tool_registry.is_some())
            .field("memory_store", &self.memory_store.is_some())
            .field("event_bus", &self.event_bus.is_some())
            .field("orchestrator", &self.orchestrator.is_some())
            .finish()
    }
}

impl Default for ScriptContext {
    fn default() -> Self {
        Self {
            agent_id: String::new(),
            globals: Default::default(),
            timeout: Duration::from_secs(30),
            fs_config: FsBridgeConfig::default(),
            net_config: NetBridgeConfig::default(),
            permissions: PermissionSet::minimal(),
            tool_registry: None,
            memory_store: None,
            event_bus: None,
            orchestrator: None,
            _resource_holders: Vec::new(),
        }
    }
}

impl ScriptContext {
    /// Create a new script context for the given agent.
    ///
    /// Initializes with default values: 30s timeout, minimal permissions,
    /// empty globals, and no bridges enabled.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_script::types::ScriptContext;
    ///
    /// let ctx = ScriptContext::new("agent-1");
    /// assert_eq!(ctx.agent_id, "agent-1");
    /// ```
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            globals: Default::default(),
            timeout: Duration::from_secs(30),
            fs_config: FsBridgeConfig::default(),
            net_config: NetBridgeConfig::default(),
            permissions: PermissionSet::minimal(),
            tool_registry: None,
            memory_store: None,
            event_bus: None,
            orchestrator: None,
            _resource_holders: Vec::new(),
        }
    }

    /// Set the tool registry for executing tools from scripts.
    ///
    /// When set, scripts can call registered tools via the tools bridge.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_script::types::ScriptContext;
    /// use claw_tools::registry::ToolRegistry;
    /// use std::sync::Arc;
    ///
    /// let registry = Arc::new(ToolRegistry::new());
    /// let ctx = ScriptContext::new("agent-1")
    ///     .with_tool_registry(registry);
    ///
    /// assert!(ctx.tool_registry.is_some());
    /// ```
    pub fn with_tool_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.tool_registry = Some(registry);
        self
    }

    /// Add a global variable accessible to the script.
    ///
    /// Globals are injected into the script environment before execution.
    /// Multiple calls with the same key will overwrite previous values.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_script::types::ScriptContext;
    /// use serde_json::json;
    ///
    /// let ctx = ScriptContext::new("agent-1")
    ///     .with_global("user_name", json!("Alice"))
    ///     .with_global("max_retries", json!(3));
    ///
    /// assert_eq!(ctx.globals.len(), 2);
    /// ```
    pub fn with_global(mut self, key: impl Into<String>, val: serde_json::Value) -> Self {
        self.globals.insert(key.into(), val);
        self
    }

    /// Set the script execution timeout.
    ///
    /// If script execution exceeds this duration, it will be terminated.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_script::types::ScriptContext;
    /// use std::time::Duration;
    ///
    /// let ctx = ScriptContext::new("agent-1")
    ///     .with_timeout(Duration::from_secs(60));
    ///
    /// assert_eq!(ctx.timeout, Duration::from_secs(60));
    /// ```
    pub fn with_timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    /// Set filesystem bridge configuration.
    ///
    /// Controls which paths the script can read from and write to.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_script::types::{ScriptContext, FsBridgeConfig};
    /// use std::path::PathBuf;
    /// use std::collections::HashSet;
    ///
    /// let fs_config = FsBridgeConfig {
    ///     allowed_paths: [PathBuf::from("/tmp")].iter().cloned().collect(),
    ///     base_dir: PathBuf::from("/tmp"),
    /// };
    ///
    /// let ctx = ScriptContext::new("agent-1")
    ///     .with_fs_config(fs_config);
    /// ```
    pub fn with_fs_config(mut self, config: FsBridgeConfig) -> Self {
        self.fs_config = config;
        self
    }

    /// Set network bridge configuration.
    ///
    /// Controls which domains and ports the script can connect to.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_script::types::{ScriptContext, NetBridgeConfig};
    ///
    /// let net_config = NetBridgeConfig::with_domains(vec!["api.example.com".to_string()]);
    /// let ctx = ScriptContext::new("agent-1")
    ///     .with_net_config(net_config);
    /// ```
    pub fn with_net_config(mut self, config: NetBridgeConfig) -> Self {
        self.net_config = config;
        self
    }

    /// Set tool permissions for script execution.
    ///
    /// Defines what operations scripts are allowed to perform.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_script::types::ScriptContext;
    /// use claw_tools::types::PermissionSet;
    ///
    /// let ctx = ScriptContext::new("agent-1")
    ///     .with_permissions(PermissionSet::minimal());
    /// ```
    pub fn with_permissions(mut self, permissions: PermissionSet) -> Self {
        self.permissions = permissions;
        self
    }

    /// Set the tool registry (alias for `with_tool_registry`).
    ///
    /// Provides a builder-style method for optional chaining.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_script::types::ScriptContext;
    /// use claw_tools::registry::ToolRegistry;
    /// use std::sync::Arc;
    ///
    /// let registry = Arc::new(ToolRegistry::new());
    /// let ctx = ScriptContext::new("agent-1")
    ///     .with_registry(registry);
    /// ```
    pub fn with_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.tool_registry = Some(registry);
        self
    }

    /// Set the memory store for the memory bridge.
    ///
    /// Allows scripts to store and retrieve memories.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_script::types::ScriptContext;
    /// use claw_memory::SqliteMemoryStore;
    /// use std::sync::Arc;
    ///
    /// // let store = Arc::new(SqliteMemoryStore::new_in_memory().unwrap());
    /// // let ctx = ScriptContext::new("agent-1").with_memory_store(store);
    /// ```
    pub fn with_memory_store(mut self, store: Arc<dyn MemoryStore>) -> Self {
        self.memory_store = Some(store);
        self
    }

    /// Set the event bus for the events bridge.
    ///
    /// Allows scripts to publish events to the runtime.
    ///
    /// # Example
    ///
    /// ```
    /// use claw_script::types::ScriptContext;
    /// use claw_runtime::EventBus;
    /// use std::sync::Arc;
    ///
    /// let bus = Arc::new(EventBus::new());
    /// let ctx = ScriptContext::new("agent-1")
    ///     .with_event_bus(bus);
    /// ```
    pub fn with_event_bus(mut self, bus: Arc<EventBus>) -> Self {
        self.event_bus = Some(bus);
        self
    }

    /// Set the agent orchestrator for the agent bridge.
    ///
    /// Allows scripts to spawn and manage other agents.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use claw_script::types::ScriptContext;
    /// use claw_runtime::{AgentOrchestrator, EventBus};
    /// use claw_pal::TokioProcessManager;
    /// use std::sync::Arc;
    ///
    /// let bus = Arc::new(EventBus::new());
    /// let pm = Arc::new(TokioProcessManager::new());
    /// let orch = Arc::new(AgentOrchestrator::new(bus, pm));
    /// let ctx = ScriptContext::new("agent-1")
    ///     .with_orchestrator(orch);
    /// ```
    pub fn with_orchestrator(mut self, orc: Arc<AgentOrchestrator>) -> Self {
        self.orchestrator = Some(orc);
        self
    }
}

/// Output from a script execution — a JSON value.
pub type ScriptValue = serde_json::Value;

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_script_lua_constructor() {
        let s = Script::lua("my-script", "return 1");
        assert_eq!(s.name, "my-script");
        assert_eq!(s.source, "return 1");
        assert_eq!(s.engine, EngineType::Lua);
    }

    #[test]
    fn test_script_context_new() {
        let ctx = ScriptContext::new("agent-1");
        assert_eq!(ctx.agent_id, "agent-1");
        assert!(ctx.globals.is_empty());
    }

    #[test]
    fn test_script_context_with_global() {
        let ctx = ScriptContext::new("agent-1")
            .with_global("x", json!(42))
            .with_global("name", json!("claw"));
        assert_eq!(ctx.globals.len(), 2);
        assert_eq!(ctx.globals["x"], json!(42));
        assert_eq!(ctx.globals["name"], json!("claw"));
    }

    #[test]
    fn test_engine_type_serialize() {
        let t = EngineType::Lua;
        let s = serde_json::to_string(&t).unwrap();
        assert_eq!(s, "\"Lua\"");
    }

    #[test]
    fn test_script_clone() {
        let s = Script::lua("test", "return true");
        let s2 = s.clone();
        assert_eq!(s2.name, "test");
        assert_eq!(s2.source, "return true");
    }

    #[test]
    fn test_script_context_default_timeout() {
        let ctx = ScriptContext::new("agent-x");
        assert_eq!(ctx.timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_script_context_with_timeout() {
        let ctx = ScriptContext::new("agent-x").with_timeout(Duration::from_millis(500));
        assert_eq!(ctx.timeout, Duration::from_millis(500));
    }

    #[test]
    fn test_script_context_default_impl() {
        let ctx = ScriptContext::default();
        assert_eq!(ctx.agent_id, "");
        assert_eq!(ctx.timeout, Duration::from_secs(30));
    }
}
