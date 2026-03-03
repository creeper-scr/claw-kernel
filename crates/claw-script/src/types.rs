use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use claw_tools::{registry::ToolRegistry, types::PermissionSet};
use serde::{Deserialize, Serialize};

/// Supported scripting engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EngineType {
    Lua,
    // JavaScript and Python reserved for future phases
}

/// A compiled/loaded script.
#[derive(Debug, Clone)]
pub struct Script {
    pub name: String,
    pub source: String,
    pub engine: EngineType,
}

impl Script {
    pub fn lua(name: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            source: source.into(),
            engine: EngineType::Lua,
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
        }
    }
}

impl ScriptContext {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            globals: Default::default(),
            timeout: Duration::from_secs(30),
            fs_config: FsBridgeConfig::default(),
            net_config: NetBridgeConfig::default(),
            permissions: PermissionSet::minimal(),
            tool_registry: None,
        }
    }

    /// Set the tool registry for executing tools from scripts.
    pub fn with_tool_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.tool_registry = Some(registry);
        self
    }

    pub fn with_global(mut self, key: impl Into<String>, val: serde_json::Value) -> Self {
        self.globals.insert(key.into(), val);
        self
    }

    pub fn with_timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    /// Set filesystem bridge configuration.
    pub fn with_fs_config(mut self, config: FsBridgeConfig) -> Self {
        self.fs_config = config;
        self
    }

    /// Set network bridge configuration.
    pub fn with_net_config(mut self, config: NetBridgeConfig) -> Self {
        self.net_config = config;
        self
    }

    /// Set tool permissions.
    pub fn with_permissions(mut self, permissions: PermissionSet) -> Self {
        self.permissions = permissions;
        self
    }

    /// Set the tool registry (builder-style for optional chaining).
    pub fn with_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.tool_registry = Some(registry);
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
