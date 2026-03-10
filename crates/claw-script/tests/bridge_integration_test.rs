//! Integration tests for bridge implementations.
//!
//! Tests are organized by bridge: dirs, events, agent.
//!
//! Note: Memory bridge tests were removed in v1.3.0 (D1 decision).
//! Memory operations are now application-layer responsibility; use
//! the `claw-memory` crate's Rust API directly.

use std::sync::Arc;

use claw_runtime::{event_bus::EventBus, events::Event, orchestrator::AgentOrchestrator};
use claw_script::{Script, ScriptContext, ScriptEngine};

#[cfg(feature = "engine-lua")]
use claw_script::LuaEngine;

use serde_json::json;

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn make_event_bus() -> Arc<EventBus> {
    Arc::new(EventBus::new())
}

fn make_orchestrator() -> Arc<AgentOrchestrator> {
    use claw_pal::TokioProcessManager;

    Arc::new(AgentOrchestrator::new(
        Arc::new(EventBus::new()),
        Arc::new(TokioProcessManager::new()),
    ))
}

// ─── Dirs Bridge Tests ────────────────────────────────────────────────────────

#[cfg(feature = "engine-lua")]
#[test]
fn test_dirs_bridge_in_lua_engine() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let engine = LuaEngine::new();
        let ctx = ScriptContext::new("test-agent");

        // dirs bridge is always registered; config_dir should return string or nil
        let result = engine
            .execute(
                &Script::lua(
                    "t",
                    "local d = dirs:config_dir(); return d == nil or type(d) == 'string'",
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result, json!(true));
    });
}

#[cfg(feature = "engine-lua")]
#[test]
fn test_dirs_bridge_all_methods_in_engine() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let engine = LuaEngine::new();
        let ctx = ScriptContext::new("test-agent");

        let result = engine
            .execute(
                &Script::lua(
                    "t",
                    r#"
                    local ok = true
                    local _ = dirs:config_dir()
                    local _ = dirs:data_dir()
                    local _ = dirs:cache_dir()
                    local _ = dirs:tools_dir()
                    local _ = dirs:scripts_dir()
                    local _ = dirs:logs_dir()
                    return true
                "#,
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result, json!(true));
    });
}

// ─── Events Bridge Tests ──────────────────────────────────────────────────────

#[cfg(feature = "engine-lua")]
#[test]
fn test_events_bridge_emit_and_subscribe() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let bus = make_event_bus();
        let engine = LuaEngine::new();
        let ctx = ScriptContext::new("agent-evt-test").with_event_bus(Arc::clone(&bus));

        // Subscribe to bus from Rust before executing
        let mut rx = bus.subscribe();

        // Emit from Lua
        engine
            .execute(
                &Script::lua("t", r#"events:emit("lua_ping", {msg = "hello"})"#),
                &ctx,
            )
            .await
            .unwrap();

        // Check the event arrived on the bus
        let event = rx.try_recv().unwrap();
        if let Event::Custom { event_type, data } = event {
            assert_eq!(event_type, "lua_ping");
            assert_eq!(data["msg"], "hello");
        } else {
            panic!("Expected Custom event, got: {:?}", event);
        }
    });
}

#[cfg(feature = "engine-lua")]
#[test]
fn test_events_bridge_on_and_poll() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let bus = make_event_bus();
        let engine = LuaEngine::new();
        let ctx = ScriptContext::new("agent-poll-test").with_event_bus(Arc::clone(&bus));

        // Emit and receive within the same script execution (same Lua + receiver instance)
        let result = engine
            .execute(
                &Script::lua(
                    "t",
                    r#"
                    local received = false
                    events:on("test_event", function(data)
                        received = true
                    end)
                    -- emit the event so the bridge's receiver gets it
                    events:emit("test_event", {value = 42})
                    -- poll to invoke the callback
                    events:poll()
                    return received
                "#,
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result, json!(true));
    });
}

// ─── Agent Bridge Tests ───────────────────────────────────────────────────────

#[cfg(feature = "engine-lua")]
#[test]
fn test_agent_bridge_spawn_and_status() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let orc = make_orchestrator();
        let engine = LuaEngine::new();
        let ctx = ScriptContext::new("parent-agent").with_orchestrator(Arc::clone(&orc));

        let result = engine
            .execute(
                &Script::lua(
                    "t",
                    r#"
                    local id = agent:spawn("worker")
                    return agent:status(id)
                "#,
                ),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result, json!("running"));
    });
}

#[cfg(feature = "engine-lua")]
#[test]
fn test_agent_bridge_auto_cleanup() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let orc = make_orchestrator();

        {
            let engine = LuaEngine::new();
            let ctx = ScriptContext::new("parent-agent").with_orchestrator(Arc::clone(&orc));

            engine
                .execute(
                    &Script::lua(
                        "t",
                        r#"
                        agent:spawn("child1")
                        agent:spawn("child2")
                    "#,
                    ),
                    &ctx,
                )
                .await
                .unwrap();
            // engine and ctx drop here; spawn_blocking task has already completed
            // so AgentBridge was dropped at the end of the blocking task
        }

        // After the blocking task ends, AgentBridge is dropped and children are cleaned up.
        assert_eq!(
            orc.agent_count(),
            0,
            "children should be cleaned up on script end"
        );
    });
}

#[cfg(feature = "engine-lua")]
#[test]
fn test_agent_bridge_kill() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let orc = make_orchestrator();
        let engine = LuaEngine::new();
        let ctx = ScriptContext::new("parent-agent").with_orchestrator(Arc::clone(&orc));

        engine
            .execute(
                &Script::lua(
                    "t",
                    r#"
                    local id = agent:spawn("temp")
                    agent:kill(id)
                "#,
                ),
                &ctx,
            )
            .await
            .unwrap();

        assert_eq!(orc.agent_count(), 0);
    });
}
