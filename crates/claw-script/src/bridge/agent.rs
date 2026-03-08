//! Agent bridge — exposes AgentOrchestrator to Lua scripts.

use std::sync::{Arc, Mutex};

use claw_runtime::{
    agent_types::{AgentConfig, AgentId, AgentStatus},
    orchestrator::AgentOrchestrator,
};
use mlua::{Lua, Result as LuaResult, UserData, UserDataMethods};

/// Agent bridge exposing AgentOrchestrator to Lua scripts.
///
/// Allows scripts to spawn, monitor, and terminate in-process agents.
/// All child agents spawned through this bridge are automatically
/// unregistered when the bridge is dropped (i.e., when the script ends).
///
/// Registered as the global `agent` table.
///
/// # Example in Lua:
/// ```lua
/// -- Spawn a child agent (in-process registration)
/// local child_id = agent:spawn("analyzer")
///
/// -- Check status
/// local status = agent:status(child_id)
/// print("Status:", status)  -- "running"
///
/// -- Terminate
/// agent:kill(child_id)
///
/// -- List all children
/// local children = agent:list()
/// for _, id in ipairs(children) do
///     print("Child:", id)
/// end
/// ```
pub struct AgentBridge {
    orchestrator: Arc<AgentOrchestrator>,
    #[allow(dead_code)]
    parent_agent_id: String,
    /// Tracks child agents spawned by this bridge for auto-cleanup.
    children: Mutex<Vec<AgentId>>,
}

impl AgentBridge {
    /// Create a new AgentBridge.
    pub fn new(orchestrator: Arc<AgentOrchestrator>, parent_agent_id: impl Into<String>) -> Self {
        Self {
            orchestrator,
            parent_agent_id: parent_agent_id.into(),
            children: Mutex::new(Vec::new()),
        }
    }
}

impl Drop for AgentBridge {
    fn drop(&mut self) {
        // Auto-cleanup: unregister all child agents when the bridge is dropped.
        if let Ok(children) = self.children.lock() {
            for id in children.iter() {
                let _ = self.orchestrator.unregister(id, "parent script ended");
            }
        }
    }
}

impl UserData for AgentBridge {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        // spawn(name) -> agent_id string
        methods.add_method("spawn", |_lua, this, name: String| {
            let config = AgentConfig::new(&name);
            let agent_id = config.agent_id.clone();

            this.orchestrator
                .register(config)
                .map_err(|e| mlua::Error::RuntimeError(format!("agent spawn error: {}", e)))?;

            // Track this child for auto-cleanup.
            this.children.lock().unwrap().push(agent_id.clone());

            Ok(agent_id.0)
        });

        // status(agent_id) -> "running" | "stopped" | "error" | "starting" | "paused" | "unknown"
        methods.add_method("status", |_lua, this, agent_id: String| {
            let id = AgentId::new(agent_id);
            let status_str = match this.orchestrator.agent_info(&id) {
                Some(info) => match info.status {
                    AgentStatus::Running => "running",
                    AgentStatus::Stopped => "stopped",
                    AgentStatus::Error => "error",
                    AgentStatus::Starting => "starting",
                    AgentStatus::Paused => "paused",
                },
                None => "unknown",
            };
            Ok(status_str.to_string())
        });

        // kill(agent_id) -> nil  (unregisters the agent)
        methods.add_method("kill", |_lua, this, agent_id: String| {
            let id = AgentId::new(&agent_id);
            this.orchestrator
                .unregister(&id, "killed by script")
                .map_err(|e| mlua::Error::RuntimeError(format!("agent kill error: {}", e)))?;

            // Remove from children tracking.
            let mut children = this.children.lock().unwrap();
            children.retain(|c| c.0 != agent_id);

            Ok(())
        });

        // list() -> table of agent_id strings
        methods.add_method("list", |lua, this, ()| {
            let children = this.children.lock().unwrap();
            let table = lua.create_table()?;
            for (i, id) in children.iter().enumerate() {
                table.raw_set(i + 1, id.0.clone())?;
            }
            Ok(table)
        });

        // info(agent_id) -> {name, status, started_at} | nil
        methods.add_method("info", |lua, this, agent_id: String| {
            let id = AgentId::new(agent_id);
            match this.orchestrator.agent_info(&id) {
                Some(info) => {
                    let tbl = lua.create_table()?;
                    tbl.set("name", info.config.name.clone())?;
                    let status_str = match info.status {
                        AgentStatus::Running => "running",
                        AgentStatus::Stopped => "stopped",
                        AgentStatus::Error => "error",
                        AgentStatus::Starting => "starting",
                        AgentStatus::Paused => "paused",
                    };
                    tbl.set("status", status_str)?;
                    tbl.set("started_at", info.started_at as i64)?;
                    Ok(mlua::Value::Table(tbl))
                }
                None => Ok(mlua::Value::Nil),
            }
        });
    }
}

/// Register the AgentBridge as a global `agent` table in the Lua instance.
pub fn register_agent(lua: &Lua, bridge: AgentBridge) -> LuaResult<()> {
    lua.globals().set("agent", bridge)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use claw_runtime::event_bus::EventBus;
    use std::sync::Arc;

    fn make_orchestrator() -> Arc<AgentOrchestrator> {
        let bus = Arc::new(EventBus::new());
        Arc::new(AgentOrchestrator::new(bus))
    }

    #[test]
    fn test_agent_bridge_spawn() {
        let orc = make_orchestrator();
        let bridge = AgentBridge::new(Arc::clone(&orc), "parent");

        let lua = Lua::new();
        register_agent(&lua, bridge).unwrap();

        let agent_id: String = lua
            .load(r#"return agent:spawn("worker")"#)
            .eval()
            .unwrap();

        assert!(!agent_id.is_empty());
        assert_eq!(orc.agent_count(), 1);
    }

    #[test]
    fn test_agent_bridge_status() {
        let orc = make_orchestrator();
        let bridge = AgentBridge::new(Arc::clone(&orc), "parent");

        let lua = Lua::new();
        register_agent(&lua, bridge).unwrap();

        lua.load(r#"_id = agent:spawn("test-agent")"#).exec().unwrap();

        let status: String = lua
            .load(r#"return agent:status(_id)"#)
            .eval()
            .unwrap();
        assert_eq!(status, "running");
    }

    #[test]
    fn test_agent_bridge_status_unknown() {
        let orc = make_orchestrator();
        let bridge = AgentBridge::new(Arc::clone(&orc), "parent");

        let lua = Lua::new();
        register_agent(&lua, bridge).unwrap();

        let status: String = lua
            .load(r#"return agent:status("nonexistent-id")"#)
            .eval()
            .unwrap();
        assert_eq!(status, "unknown");
    }

    #[test]
    fn test_agent_bridge_kill() {
        let orc = make_orchestrator();
        let bridge = AgentBridge::new(Arc::clone(&orc), "parent");

        let lua = Lua::new();
        register_agent(&lua, bridge).unwrap();

        lua.load(r#"
            _id = agent:spawn("ephemeral")
            agent:kill(_id)
        "#)
        .exec()
        .unwrap();

        assert_eq!(orc.agent_count(), 0);
    }

    #[test]
    fn test_agent_bridge_list() {
        let orc = make_orchestrator();
        let bridge = AgentBridge::new(Arc::clone(&orc), "parent");

        let lua = Lua::new();
        register_agent(&lua, bridge).unwrap();

        lua.load(r#"
            agent:spawn("a")
            agent:spawn("b")
        "#)
        .exec()
        .unwrap();

        let count: i64 = lua
            .load(r#"
                local children = agent:list()
                return #children
            "#)
            .eval()
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn test_agent_bridge_auto_cleanup_on_drop() {
        let orc = make_orchestrator();

        {
            let bridge = AgentBridge::new(Arc::clone(&orc), "parent");
            let lua = Lua::new();
            register_agent(&lua, bridge).unwrap();

            lua.load(r#"
                agent:spawn("temp1")
                agent:spawn("temp2")
            "#)
            .exec()
            .unwrap();

            assert_eq!(orc.agent_count(), 2);
            // bridge (and lua) dropped here
        }

        // After drop, children should be cleaned up.
        assert_eq!(orc.agent_count(), 0, "auto-cleanup should remove all children");
    }

    #[test]
    fn test_agent_bridge_info() {
        let orc = make_orchestrator();
        let bridge = AgentBridge::new(Arc::clone(&orc), "parent");

        let lua = Lua::new();
        register_agent(&lua, bridge).unwrap();

        lua.load(r#"_id = agent:spawn("my-agent")"#).exec().unwrap();

        let has_info: bool = lua
            .load(r#"
                local info = agent:info(_id)
                return info ~= nil and info.status == "running"
            "#)
            .eval()
            .unwrap();
        assert!(has_info);
    }
}
