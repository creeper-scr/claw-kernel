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

/// Execution context passed to scripts.
#[derive(Debug, Clone, Default)]
pub struct ScriptContext {
    /// Agent ID executing the script.
    pub agent_id: String,
    /// Script-accessible global variables (JSON values).
    pub globals: std::collections::HashMap<String, serde_json::Value>,
}

impl ScriptContext {
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            globals: Default::default(),
        }
    }

    pub fn with_global(mut self, key: impl Into<String>, val: serde_json::Value) -> Self {
        self.globals.insert(key.into(), val);
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
}
