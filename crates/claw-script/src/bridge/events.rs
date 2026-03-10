//! Events bridge — subscribes to / emits events via the EventBus.

use std::sync::{Arc, Mutex};

use claw_runtime::{event_bus::EventBus, events::Event, EventReceiver};
use mlua::{Lua, Result as LuaResult, UserData, UserDataMethods};
use serde_json::{json, Value as JsonValue};

use crate::bridge::conversion::{json_to_lua, lua_to_json};

/// Events bridge exposing EventBus to Lua scripts.
///
/// Registered as the global `events` table.
///
/// Usage model:
/// - `on(event_type, callback)` — register a callback for an event type
/// - `once(event_type, callback)` — register a one-shot callback
/// - `emit(event_type, data)` — publish a custom event
/// - `poll()` — process pending events and invoke registered callbacks
///
/// # Event type strings
/// System events: "agent_started", "agent_stopped", "tool_called", "shutdown", etc.
/// Custom events: any string passed to `emit()`.
/// Wildcard: "*" matches all events.
///
/// # Example in Lua:
/// ```lua
/// events:on("agent_started", function(data)
///     print("Agent started:", data.agent_id)
/// end)
///
/// events:emit("task_done", {status = "ok"})
///
/// -- In the main loop:
/// events:poll()
/// ```
pub struct EventsBridge {
    event_bus: Arc<EventBus>,
    /// Agent identifier; retained for future use (e.g., filtering own events).
    #[allow(dead_code)]
    agent_id: String,
    /// EventReceiver wrapped in Mutex for Send+Sync compatibility.
    rx: Mutex<EventReceiver>,
    /// Lua table (stored via RegistryKey) mapping event_type -> [{fn, once}].
    callbacks_key: mlua::RegistryKey,
}

impl EventsBridge {
    /// Create a new EventsBridge.
    ///
    /// The Lua instance must be the same one that will execute scripts.
    pub fn new(
        lua: &Lua,
        event_bus: Arc<EventBus>,
        agent_id: impl Into<String>,
    ) -> LuaResult<Self> {
        let rx = event_bus.subscribe();
        let callbacks_tbl = lua.create_table()?;
        let callbacks_key = lua.create_registry_value(callbacks_tbl)?;
        Ok(Self {
            event_bus,
            agent_id: agent_id.into(),
            rx: Mutex::new(rx),
            callbacks_key,
        })
    }
}

/// Convert an `Event` to a (type_string, data) pair for Lua.
///
/// Returns `(event_type_string, json_data)`. For `Custom` events the
/// event_type_string is the user-supplied type (not "custom"), so that
/// `poll()` can match it directly against registered handler keys.
fn event_to_parts(event: &Event) -> (String, JsonValue) {
    match event {
        Event::AgentStarted { agent_id } => (
            "agent_started".to_string(),
            json!({ "agent_id": agent_id.as_str() }),
        ),
        Event::AgentStopped { agent_id, reason } => (
            "agent_stopped".to_string(),
            json!({ "agent_id": agent_id.as_str(), "reason": reason }),
        ),
        Event::LlmRequestStarted { agent_id, provider } => (
            "llm_request_started".to_string(),
            json!({ "agent_id": agent_id.as_str(), "provider": provider }),
        ),
        Event::LlmRequestCompleted {
            agent_id,
            prompt_tokens,
            completion_tokens,
        } => (
            "llm_request_completed".to_string(),
            json!({
                "agent_id": agent_id.as_str(),
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens
            }),
        ),
        Event::MessageReceived {
            agent_id,
            channel,
            message_type,
        } => (
            "message_received".to_string(),
            json!({
                "agent_id": agent_id.as_str(),
                "channel": channel,
                "message_type": message_type
            }),
        ),
        Event::ToolCalled {
            agent_id,
            tool_name,
            call_id,
        } => (
            "tool_called".to_string(),
            json!({
                "agent_id": agent_id.as_str(),
                "tool_name": tool_name,
                "call_id": call_id
            }),
        ),
        Event::ToolResult {
            agent_id,
            tool_name,
            call_id,
            success,
        } => (
            "tool_result".to_string(),
            json!({
                "agent_id": agent_id.as_str(),
                "tool_name": tool_name,
                "call_id": call_id,
                "success": success
            }),
        ),
        Event::ContextWindowApproachingLimit {
            agent_id,
            token_count,
            token_limit,
        } => (
            "context_window_approaching_limit".to_string(),
            json!({
                "agent_id": agent_id.as_str(),
                "token_count": token_count,
                "token_limit": token_limit
            }),
        ),
        Event::MemoryArchiveComplete {
            agent_id,
            archived_count,
        } => (
            "memory_archive_complete".to_string(),
            json!({
                "agent_id": agent_id.as_str(),
                "archived_count": archived_count
            }),
        ),
        Event::ModeChanged {
            agent_id,
            to_power_mode,
        } => (
            "mode_changed".to_string(),
            json!({
                "agent_id": agent_id.as_str(),
                "to_power_mode": to_power_mode
            }),
        ),
        Event::Shutdown => ("shutdown".to_string(), json!({})),
        Event::Extension(_) => ("extension".to_string(), json!({})),
        // For Custom events: use the user-supplied event_type directly so that
        // `poll()` can match handlers registered under that string.
        Event::Custom { event_type, data } => (event_type.clone(), data.clone()),
        // #[non_exhaustive] requires a wildcard arm for exhaustive matching.
        _ => ("unknown".to_string(), json!({})),
    }
}

/// Convert a `serde_json::Value` to a `mlua::Value`.
fn json_to_lua_val<'lua>(lua: &'lua Lua, val: &JsonValue) -> LuaResult<mlua::Value<'lua>> {
    json_to_lua(lua, val, 0)
}

impl UserData for EventsBridge {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        // on(event_type, callback) — register a persistent callback.
        methods.add_method(
            "on",
            |lua, this, (event_type, callback): (String, mlua::Function)| {
                let tbl: mlua::Table = lua.registry_value(&this.callbacks_key)?;

                let arr: mlua::Table = match tbl.get::<_, mlua::Value>(event_type.as_str())? {
                    mlua::Value::Table(t) => t,
                    _ => {
                        let t = lua.create_table()?;
                        tbl.set(event_type.as_str(), t.clone())?;
                        t
                    }
                };

                let entry = lua.create_table()?;
                entry.set("fn", callback)?;
                entry.set("once", false)?;
                arr.push(entry)?;
                Ok(())
            },
        );

        // once(event_type, callback) — register a one-shot callback.
        methods.add_method(
            "once",
            |lua, this, (event_type, callback): (String, mlua::Function)| {
                let tbl: mlua::Table = lua.registry_value(&this.callbacks_key)?;

                let arr: mlua::Table = match tbl.get::<_, mlua::Value>(event_type.as_str())? {
                    mlua::Value::Table(t) => t,
                    _ => {
                        let t = lua.create_table()?;
                        tbl.set(event_type.as_str(), t.clone())?;
                        t
                    }
                };

                let entry = lua.create_table()?;
                entry.set("fn", callback)?;
                entry.set("once", true)?;
                arr.push(entry)?;
                Ok(())
            },
        );

        // emit(event_type, data) — publish a Custom event to the EventBus.
        methods.add_method(
            "emit",
            |_lua, this, (event_type, data): (String, Option<mlua::Value>)| {
                let json_data = match data {
                    Some(v) => lua_to_json(v),
                    None => json!({}),
                };
                let event = Event::Custom {
                    event_type,
                    data: json_data,
                };
                let _ = this.event_bus.publish(event);
                Ok(())
            },
        );

        // poll() — drain pending events and invoke registered callbacks.
        methods.add_method("poll", |lua, this, ()| {
            // Step 1: drain pending events (brief mutex hold).
            let pending: Vec<Event> = {
                let mut rx = this.rx.lock().unwrap();
                let mut events = Vec::new();
                while let Ok(event) = rx.try_recv() {
                    events.push(event);
                }
                events
            };

            if pending.is_empty() {
                return Ok(());
            }

            let callbacks_tbl: mlua::Table = lua.registry_value(&this.callbacks_key)?;

            for event in &pending {
                let (type_str, data) = event_to_parts(event);
                let lua_data = json_to_lua_val(lua, &data)?;

                // Match against the specific event type AND the wildcard "*".
                let match_keys: Vec<&str> = vec![type_str.as_str(), "*"];

                for &match_key in &match_keys {
                    let callbacks: mlua::Table =
                        match callbacks_tbl.get::<_, mlua::Value>(match_key)? {
                            mlua::Value::Table(t) => t,
                            _ => continue,
                        };

                    let len = callbacks.raw_len();

                    for i in 1..=(len as i64) {
                        let entry: mlua::Table = match callbacks.raw_get::<i64, mlua::Value>(i)? {
                            mlua::Value::Table(t) => t,
                            _ => continue,
                        };

                        // Check if this entry is still alive (fn may be nil for spent once-callbacks).
                        let func: Option<mlua::Function> =
                            match entry.get::<_, mlua::Value>("fn")? {
                                mlua::Value::Function(f) => Some(f),
                                _ => None,
                            };
                        let once: bool = entry.get::<_, bool>("once").unwrap_or(false);

                        if let Some(func) = func {
                            // Invoke callback; ignore errors to avoid stopping event processing.
                            let _ = func.call::<_, ()>(lua_data.clone());

                            // Mark one-shot callbacks as spent by setting fn to nil.
                            if once {
                                entry.set("fn", mlua::Value::Nil)?;
                            }
                        }
                    }
                }
            }

            Ok(())
        });
    }
}

/// Register the EventsBridge as a global `events` table in the Lua instance.
pub fn register_events(lua: &Lua, bridge: EventsBridge) -> LuaResult<()> {
    lua.globals().set("events", bridge)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_bus() -> Arc<EventBus> {
        Arc::new(EventBus::new())
    }

    #[test]
    fn test_events_bridge_register() {
        let bus = make_bus();
        let lua = Lua::new();
        let bridge = EventsBridge::new(&lua, bus, "agent-1").unwrap();
        register_events(&lua, bridge).unwrap();

        let result: bool = lua.load("return events ~= nil").eval().unwrap();
        assert!(result);
    }

    #[test]
    fn test_events_bridge_on_and_poll() {
        let bus = make_bus();
        let lua = Lua::new();
        let bridge = EventsBridge::new(&lua, Arc::clone(&bus), "agent-1").unwrap();
        register_events(&lua, bridge).unwrap();

        // Register a callback.
        lua.load(
            r#"
            _fired = false
            events:on("custom_event", function(data)
                _fired = true
            end)
        "#,
        )
        .exec()
        .unwrap();

        // Publish an event from Rust.
        bus.publish(Event::Custom {
            event_type: "custom_event".to_string(),
            data: json!({"msg": "hello"}),
        });

        // Poll to process.
        lua.load("events:poll()").exec().unwrap();

        let fired: bool = lua.load("return _fired").eval().unwrap();
        assert!(fired, "callback should have been invoked");
    }

    #[test]
    fn test_events_bridge_once() {
        let bus = make_bus();
        let lua = Lua::new();
        let bridge = EventsBridge::new(&lua, Arc::clone(&bus), "agent-1").unwrap();
        register_events(&lua, bridge).unwrap();

        lua.load(
            r#"
            _count = 0
            events:once("my_event", function(data)
                _count = _count + 1
            end)
        "#,
        )
        .exec()
        .unwrap();

        // Publish twice.
        bus.publish(Event::Custom {
            event_type: "my_event".to_string(),
            data: json!({}),
        });
        bus.publish(Event::Custom {
            event_type: "my_event".to_string(),
            data: json!({}),
        });

        // Poll once to process both events.
        lua.load("events:poll()").exec().unwrap();

        let count: i64 = lua.load("return _count").eval().unwrap();
        assert_eq!(count, 1, "once callback should fire exactly once");
    }

    #[test]
    fn test_events_bridge_emit() {
        let bus = make_bus();
        let lua = Lua::new();
        let bridge = EventsBridge::new(&lua, Arc::clone(&bus), "agent-1").unwrap();
        register_events(&lua, bridge).unwrap();

        // Subscribe to listen for the custom event.
        let mut rx = bus.subscribe();

        // Emit from Lua.
        lua.load(r#"events:emit("lua_event", {key = "value"})"#)
            .exec()
            .unwrap();

        // The event should be on the bus.
        let event = rx.try_recv();
        assert!(event.is_ok(), "event should have been published");
        if let Ok(Event::Custom { event_type, data }) = event {
            assert_eq!(event_type, "lua_event");
            assert_eq!(data["key"], "value");
        } else {
            panic!("expected Custom event");
        }
    }

    #[test]
    fn test_events_bridge_wildcard() {
        let bus = make_bus();
        let lua = Lua::new();
        let bridge = EventsBridge::new(&lua, Arc::clone(&bus), "agent-1").unwrap();
        register_events(&lua, bridge).unwrap();

        lua.load(
            r#"
            _wildcard_count = 0
            events:on("*", function(data)
                _wildcard_count = _wildcard_count + 1
            end)
        "#,
        )
        .exec()
        .unwrap();

        bus.publish(Event::Custom {
            event_type: "event_a".to_string(),
            data: json!({}),
        });
        bus.publish(Event::Custom {
            event_type: "event_b".to_string(),
            data: json!({}),
        });

        lua.load("events:poll()").exec().unwrap();

        let count: i64 = lua.load("return _wildcard_count").eval().unwrap();
        assert_eq!(count, 2, "wildcard should catch all events");
    }

    #[test]
    fn test_events_bridge_poll_empty() {
        let bus = make_bus();
        let lua = Lua::new();
        let bridge = EventsBridge::new(&lua, bus, "agent-1").unwrap();
        register_events(&lua, bridge).unwrap();

        // poll with no pending events should not error.
        lua.load("events:poll()").exec().unwrap();
    }

    #[test]
    fn test_events_bridge_system_event_poll() {
        let bus = make_bus();
        let lua = Lua::new();
        let bridge = EventsBridge::new(&lua, Arc::clone(&bus), "agent-1").unwrap();
        register_events(&lua, bridge).unwrap();

        lua.load(
            r#"
            _shutdown_fired = false
            events:on("shutdown", function(data)
                _shutdown_fired = true
            end)
        "#,
        )
        .exec()
        .unwrap();

        bus.publish(Event::Shutdown);
        lua.load("events:poll()").exec().unwrap();

        let fired: bool = lua.load("return _shutdown_fired").eval().unwrap();
        assert!(fired, "shutdown callback should have been invoked");
    }

    #[test]
    fn test_events_bridge_data_passed_to_callback() {
        let bus = make_bus();
        let lua = Lua::new();
        let bridge = EventsBridge::new(&lua, Arc::clone(&bus), "agent-1").unwrap();
        register_events(&lua, bridge).unwrap();

        lua.load(
            r#"
            _received_value = nil
            events:on("data_event", function(data)
                _received_value = data.value
            end)
        "#,
        )
        .exec()
        .unwrap();

        bus.publish(Event::Custom {
            event_type: "data_event".to_string(),
            data: json!({"value": 42}),
        });

        lua.load("events:poll()").exec().unwrap();

        let value: i64 = lua.load("return _received_value").eval().unwrap();
        assert_eq!(value, 42, "data should be passed to callback");
    }
}
