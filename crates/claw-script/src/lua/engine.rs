use async_trait::async_trait;
use mlua::{Lua, Value as LuaValue};
use serde_json::{json, Value as JsonValue};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::{
    bridge::{
        agent::AgentBridge, dirs::DirsBridge, memory::MemoryBridge, register_agent, register_dirs,
        register_events, register_fs, register_memory, register_net, register_tools,
        tools::CallerContext, EventsBridge, FsBridge, NetBridge, ToolsBridge,
    },
    error::{CompileError, ScriptError},
    traits::ScriptEngine,
    types::{Script, ScriptContext, ScriptValue},
};

/// Default maximum recursion depth for JSON <-> Lua conversion.
const DEFAULT_MAX_RECURSION_DEPTH: u32 = 32;

/// Lua script engine backed by mlua (Lua 5.4).
///
/// Each execution creates a fresh Lua instance to guarantee isolation and
/// thread safety. Calls to mlua's synchronous API are dispatched via
/// `tokio::task::spawn_blocking` so the async executor is never blocked.
pub struct LuaEngine {
    /// Maximum recursion depth for JSON <-> Lua conversion.
    max_recursion_depth: u32,
}

impl LuaEngine {
    /// Create a new LuaEngine with default configuration.
    ///
    /// Default recursion depth is 32 levels.
    pub fn new() -> Self {
        Self {
            max_recursion_depth: DEFAULT_MAX_RECURSION_DEPTH,
        }
    }

    /// Create a new LuaEngine with a custom maximum recursion depth.
    ///
    /// # Arguments
    /// * `depth` - Maximum recursion depth for JSON <-> Lua conversion
    pub fn with_max_recursion_depth(depth: u32) -> Self {
        Self {
            max_recursion_depth: depth,
        }
    }
}

impl Default for LuaEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert a mlua `Value` into a `serde_json::Value`.
///
/// Tables are introspected: if `raw_len() > 0` and all 1..=n keys are present
/// the table is treated as a JSON array, otherwise as a JSON object.
///
/// Recursion is depth-limited to prevent stack overflows.
/// If the limit is exceeded, returns a special error marker string.
fn lua_to_json(val: LuaValue, depth: u32, max_depth: u32) -> JsonValue {
    if depth > max_depth {
        return JsonValue::String(format!(
            "[Error: Recursion limit exceeded (max {} levels)]",
            max_depth
        ));
    }
    match val {
        LuaValue::Nil => JsonValue::Null,
        LuaValue::Boolean(b) => json!(b),
        LuaValue::Integer(i) => json!(i),
        LuaValue::Number(f) => json!(f),
        LuaValue::String(s) => json!(s.to_str().unwrap_or("")),
        LuaValue::Table(t) => {
            // Detect array vs. object: if raw_len > 0 and every index 1..=n
            // is present, treat as a JSON array.
            let len = t.raw_len();
            if len > 0 {
                let arr: Vec<JsonValue> = (1..=(len as i64))
                    .filter_map(|i| t.raw_get::<i64, LuaValue>(i).ok())
                    .map(|v| lua_to_json(v, depth + 1, max_depth))
                    .collect();
                if arr.len() == len {
                    return JsonValue::Array(arr);
                }
            }
            // Otherwise treat as an object.
            let mut map = serde_json::Map::new();
            for (k, v) in t.pairs::<LuaValue, LuaValue>().flatten() {
                let key = match k {
                    LuaValue::String(s) => s.to_str().unwrap_or("").to_string(),
                    LuaValue::Integer(i) => i.to_string(),
                    _ => continue,
                };
                map.insert(key, lua_to_json(v, depth + 1, max_depth));
            }
            JsonValue::Object(map)
        }
        _ => JsonValue::Null,
    }
}

/// Convert a `serde_json::Value` into a mlua `Value`.
///
/// Recursion is depth-limited to prevent stack overflows.
/// Returns `Err(ScriptError::RecursionLimitExceeded)` if the limit is exceeded.
fn json_to_lua<'lua>(
    lua: &'lua Lua,
    val: &JsonValue,
    depth: u32,
    max_depth: u32,
) -> Result<LuaValue<'lua>, ScriptError> {
    if depth > max_depth {
        return Err(ScriptError::RecursionLimitExceeded(max_depth));
    }
    let lval = match val {
        JsonValue::Null => LuaValue::Nil,
        JsonValue::Bool(b) => LuaValue::Boolean(*b),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                LuaValue::Integer(i)
            } else {
                LuaValue::Number(n.as_f64().unwrap_or(0.0))
            }
        }
        JsonValue::String(s) => LuaValue::String(
            lua.create_string(s.as_bytes())
                .map_err(|e| ScriptError::Runtime(format!("failed to create lua string: {}", e)))?,
        ),
        JsonValue::Array(arr) => {
            let table = lua
                .create_table()
                .map_err(|e| ScriptError::Runtime(format!("failed to create lua table: {}", e)))?;
            for (i, elem) in arr.iter().enumerate() {
                table
                    .raw_set(i + 1, json_to_lua(lua, elem, depth + 1, max_depth)?)
                    .map_err(|e| {
                        ScriptError::Runtime(format!("failed to set table value: {}", e))
                    })?;
            }
            LuaValue::Table(table)
        }
        JsonValue::Object(map) => {
            let table = lua
                .create_table()
                .map_err(|e| ScriptError::Runtime(format!("failed to create lua table: {}", e)))?;
            for (k, v) in map {
                table
                    .raw_set(k.as_str(), json_to_lua(lua, v, depth + 1, max_depth)?)
                    .map_err(|e| {
                        ScriptError::Runtime(format!("failed to set table value: {}", e))
                    })?;
            }
            LuaValue::Table(table)
        }
    };
    Ok(lval)
}

#[async_trait]
impl ScriptEngine for LuaEngine {
    fn engine_type(&self) -> &str {
        "lua"
    }

    async fn execute(
        &self,
        script: &Script,
        ctx: &ScriptContext,
    ) -> Result<ScriptValue, ScriptError> {
        let source = script.source.clone();
        let agent_id = ctx.agent_id.clone();
        let globals_map = ctx.globals.clone();
        let timeout_dur = ctx.timeout;
        let fs_config = ctx.fs_config.clone();
        let net_config = ctx.net_config.clone();
        let tool_registry = ctx.tool_registry.clone();
        let permissions = ctx.permissions.clone();
        let max_recursion_depth = self.max_recursion_depth;
        let memory_store = ctx.memory_store.clone();
        let event_bus = ctx.event_bus.clone();
        let orchestrator = ctx.orchestrator.clone();

        // Cancellation flag shared between timeout watcher and Lua hook
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_in_task = Arc::clone(&cancelled);

        let task = tokio::task::spawn_blocking(move || -> Result<ScriptValue, ScriptError> {
            let lua = Lua::new();

            // Set up a hook that checks the cancellation flag every N instructions
            // This allows infinite loops to be interrupted
            lua.set_hook(
                mlua::HookTriggers::new().every_nth_instruction(1000),
                move |_, _| {
                    if cancelled_in_task.load(Ordering::Relaxed) {
                        Err(mlua::Error::RuntimeError(
                            "script execution timeout".to_string(),
                        ))
                    } else {
                        Ok(())
                    }
                },
            );

            // Inject agent_id global
            lua.globals()
                .set("agent_id", agent_id.clone())
                .map_err(|e| ScriptError::Runtime(e.to_string()))?;

            // Inject caller-supplied globals
            for (key, val) in &globals_map {
                let lua_val = json_to_lua(&lua, val, 0, max_recursion_depth)?;
                lua.globals()
                    .set(key.as_str(), lua_val)
                    .map_err(|e| ScriptError::Runtime(e.to_string()))?;
            }

            // Register FS bridge
            let fs_bridge = if fs_config.allowed_paths.is_empty() {
                FsBridge::empty()
            } else {
                FsBridge::new(fs_config.allowed_paths, fs_config.base_dir)
            };
            register_fs(&lua, fs_bridge).map_err(|e| {
                ScriptError::Runtime(format!("Failed to register fs bridge: {}", e))
            })?;

            // Register Net bridge
            let net_bridge = if net_config.allowed_domains.is_empty() {
                NetBridge::new()
            } else {
                let mut bridge = NetBridge::with_domains(net_config.allowed_domains);
                if !net_config.allowed_ports.is_empty() {
                    bridge = bridge.with_ports(net_config.allowed_ports);
                }
                bridge = bridge.with_loopback(net_config.allow_loopback);
                bridge =
                    bridge.with_timeout(std::time::Duration::from_secs(net_config.timeout_secs));
                bridge
            };
            register_net(&lua, net_bridge).map_err(|e| {
                ScriptError::Runtime(format!("Failed to register net bridge: {}", e))
            })?;

            // Register Tools bridge if registry is provided
            if let Some(registry) = tool_registry {
                let caller_context = CallerContext::new(agent_id.clone(), permissions);
                let tools_bridge = ToolsBridge::new(registry, caller_context);
                register_tools(&lua, tools_bridge).map_err(|e| {
                    ScriptError::Runtime(format!("Failed to register tools bridge: {}", e))
                })?;
            }

            // Register Dirs bridge (always available)
            register_dirs(&lua, DirsBridge).map_err(|e| {
                ScriptError::Runtime(format!("Failed to register dirs bridge: {}", e))
            })?;

            // Register Memory bridge if store is provided
            if let Some(store) = memory_store {
                let memory_bridge = MemoryBridge::new(store, agent_id.clone());
                register_memory(&lua, memory_bridge).map_err(|e| {
                    ScriptError::Runtime(format!("Failed to register memory bridge: {}", e))
                })?;
            }

            // Register Events bridge if event bus is provided
            if let Some(bus) = event_bus {
                let events_bridge =
                    EventsBridge::new(&lua, bus, agent_id.clone()).map_err(|e| {
                        ScriptError::Runtime(format!("Failed to create events bridge: {}", e))
                    })?;
                register_events(&lua, events_bridge).map_err(|e| {
                    ScriptError::Runtime(format!("Failed to register events bridge: {}", e))
                })?;
            }

            // Register Agent bridge if orchestrator is provided
            if let Some(orc) = orchestrator {
                let agent_bridge = AgentBridge::new(orc, agent_id);
                register_agent(&lua, agent_bridge).map_err(|e| {
                    ScriptError::Runtime(format!("Failed to register agent bridge: {}", e))
                })?;
            }

            // Load and evaluate the script
            let chunk = lua.load(&source);
            let lua_result: LuaValue = chunk
                .eval()
                .map_err(|e| ScriptError::Runtime(e.to_string()))?;

            Ok(lua_to_json(lua_result, 0, max_recursion_depth))
        });

        // Use a separate timeout task to set the cancellation flag
        // This ensures the Lua hook can detect timeout and terminate gracefully
        let timeout_handle = tokio::spawn(async move {
            tokio::time::sleep(timeout_dur).await;
            cancelled.store(true, Ordering::Relaxed);
        });

        // Wait for the Lua task to complete
        let result = task
            .await
            .map_err(|e| ScriptError::Runtime(e.to_string()))?;

        // Cancel the timeout task if Lua completed first
        timeout_handle.abort();

        result
    }

    fn validate(&self, script: &Script) -> Result<(), ScriptError> {
        let lua = Lua::new();
        lua.load(&script.source)
            .into_function()
            .map(|_| ())
            .map_err(|e| ScriptError::Compile(CompileError::Syntax(e.to_string())))
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Script, ScriptContext};
    use serde_json::json;
    use std::time::Duration;

    fn engine() -> LuaEngine {
        LuaEngine::new()
    }

    fn default_ctx() -> ScriptContext {
        ScriptContext::new("test-agent")
    }

    #[tokio::test]
    async fn test_lua_engine_execute_number() {
        let result = engine()
            .execute(&Script::lua("t", "return 42"), &default_ctx())
            .await
            .unwrap();
        assert_eq!(result, json!(42));
    }

    #[tokio::test]
    async fn test_lua_engine_execute_string() {
        let result = engine()
            .execute(&Script::lua("t", r#"return "hello""#), &default_ctx())
            .await
            .unwrap();
        assert_eq!(result, json!("hello"));
    }

    #[tokio::test]
    async fn test_lua_engine_execute_boolean() {
        let result = engine()
            .execute(&Script::lua("t", "return true"), &default_ctx())
            .await
            .unwrap();
        assert_eq!(result, json!(true));
    }

    #[tokio::test]
    async fn test_lua_engine_execute_nil() {
        let result = engine()
            .execute(&Script::lua("t", "return nil"), &default_ctx())
            .await
            .unwrap();
        assert!(result.is_null());
    }

    #[tokio::test]
    async fn test_lua_engine_execute_nil_body() {
        // Script with no explicit return — should not error
        let result = engine()
            .execute(&Script::lua("t", "local x = 1 + 1"), &default_ctx())
            .await;
        assert!(result.is_ok());
    }

    #[test]
    fn test_lua_engine_validate_valid() {
        let result = engine().validate(&Script::lua("t", "return 1 + 1"));
        assert!(result.is_ok());
    }

    #[test]
    fn test_lua_engine_validate_invalid() {
        let result = engine().validate(&Script::lua("t", "return @@@ invalid!!!"));
        assert!(matches!(result, Err(ScriptError::Compile(_))));
    }

    #[tokio::test]
    async fn test_lua_engine_global_injection() {
        let ctx = ScriptContext::new("agent-1").with_global("score", json!(99));
        let result = engine()
            .execute(&Script::lua("t", "return score"), &ctx)
            .await
            .unwrap();
        assert_eq!(result, json!(99));
    }

    #[tokio::test]
    async fn test_lua_engine_agent_id_injection() {
        let ctx = ScriptContext::new("my-agent");
        let result = engine()
            .execute(&Script::lua("t", "return agent_id"), &ctx)
            .await
            .unwrap();
        assert_eq!(result, json!("my-agent"));
    }

    #[tokio::test]
    async fn test_lua_engine_error_propagation() {
        let result = engine()
            .execute(&Script::lua("t", r#"error("boom")"#), &default_ctx())
            .await;
        assert!(matches!(result, Err(ScriptError::Runtime(_))));
        if let Err(ScriptError::Runtime(msg)) = result {
            assert!(msg.contains("boom"));
        }
    }

    // ── 3A: 超时测试 ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn test_lua_engine_timeout() {
        let ctx = ScriptContext::new("test-agent").with_timeout(Duration::from_millis(200));
        let result = engine()
            .execute(&Script::lua("t", "while true do end"), &ctx)
            .await;
        // Timeout is detected via hook and returned as Runtime error with "timeout" message
        assert!(
            matches!(result, Err(ScriptError::Runtime(ref msg)) if msg.contains("timeout")),
            "expected Runtime timeout error, got: {result:?}"
        );
    }

    // ── 4A: 嵌套对象/数组注入及 Table 返回 ───────────────────────────────────

    #[tokio::test]
    async fn test_lua_engine_inject_nested_object() {
        let ctx = ScriptContext::new("test-agent").with_global("a", json!({"b": 42}));
        let result = engine()
            .execute(&Script::lua("t", "return a.b"), &ctx)
            .await
            .unwrap();
        assert_eq!(result, json!(42));
    }

    #[tokio::test]
    async fn test_lua_engine_inject_array() {
        let ctx = ScriptContext::new("test-agent").with_global("arr", json!([1, 2, 3]));
        let result = engine()
            .execute(&Script::lua("t", "return arr[2]"), &ctx)
            .await
            .unwrap();
        assert_eq!(result, json!(2));
    }

    #[tokio::test]
    async fn test_lua_engine_return_table_as_object() {
        let result = engine()
            .execute(
                &Script::lua("t", "local t = {}; t.x = 1; t.y = 2; return t"),
                &default_ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_object(), "expected JSON object, got: {result}");
        assert_eq!(result["x"], json!(1));
        assert_eq!(result["y"], json!(2));
    }

    // ── 5A: 递归深度限制测试 ─────────────────────────────────────────────────

    #[test]
    fn test_lua_engine_default_recursion_depth() {
        let engine = LuaEngine::new();
        assert_eq!(engine.max_recursion_depth, DEFAULT_MAX_RECURSION_DEPTH);
    }

    #[test]
    fn test_lua_engine_custom_recursion_depth() {
        let engine = LuaEngine::with_max_recursion_depth(64);
        assert_eq!(engine.max_recursion_depth, 64);
    }

    #[tokio::test]
    async fn test_lua_engine_recursion_limit_in_json_to_lua() {
        // Create a deeply nested JSON structure that exceeds default limit
        let mut deep_json = json!(1);
        for _ in 0..40 {
            deep_json = json!({"nested": deep_json});
        }

        let ctx = ScriptContext::new("test-agent").with_global("deep", deep_json);
        let result = engine()
            .execute(&Script::lua("t", "return deep"), &ctx)
            .await;

        assert!(matches!(
            result,
            Err(ScriptError::RecursionLimitExceeded(_))
        ));
    }

    #[tokio::test]
    async fn test_lua_engine_recursion_limit_in_lua_to_json() {
        // Create a Lua script that returns a deeply nested table
        // Default limit is 32, so 40 levels will exceed it
        let script = r#"
            local t = {}
            local current = t
            for i = 1, 40 do
                current.nested = {}
                current = current.nested
            end
            current.value = 42
            return t
        "#;

        let result = engine()
            .execute(&Script::lua("t", script), &default_ctx())
            .await
            .unwrap();

        // The result should be an object with nested structure
        assert!(result.is_object());

        // Navigate through the nested structure to find where recursion limit kicks in
        // At depth 33, the conversion should return an error marker string
        let mut current = &result["nested"];
        let mut depth = 1;

        while current.is_object() && !current["nested"].is_null() {
            current = &current["nested"];
            depth += 1;
            // Safety check to avoid infinite loop in test
            assert!(depth < 50, "Navigation went too deep without finding limit");
        }

        // The recursion limit should have been hit (default is 32)
        // So we should have found a string marker at depth 33
        assert!(
            current.is_string(),
            "Expected string error marker at depth {}, got: {:?}",
            depth,
            current
        );
        assert!(
            current
                .as_str()
                .unwrap()
                .contains("Error: Recursion limit exceeded"),
            "Error marker should indicate recursion limit: {:?}",
            current
        );
    }

    #[tokio::test]
    async fn test_lua_engine_custom_recursion_depth_execution() {
        // Create a moderately nested JSON structure
        let mut deep_json = json!(1);
        for _ in 0..20 {
            deep_json = json!({"nested": deep_json});
        }

        // Default engine should handle 20 levels
        let ctx = ScriptContext::new("test-agent").with_global("deep", deep_json.clone());
        let result = engine()
            .execute(&Script::lua("t", "return deep"), &ctx)
            .await;
        assert!(result.is_ok());

        // Engine with lower limit should fail
        let limited_engine = LuaEngine::with_max_recursion_depth(10);
        let ctx = ScriptContext::new("test-agent").with_global("deep", deep_json);
        let result = limited_engine
            .execute(&Script::lua("t", "return deep"), &ctx)
            .await;
        assert!(matches!(
            result,
            Err(ScriptError::RecursionLimitExceeded(_))
        ));
    }

    // ─── 递归深度限制边界测试 (Agent 4: Red Phase) ────────────────────────────

    /// Test: lua_to_json 超过自定义深度限制时正确处理
    ///
    /// 验证当 Lua 表嵌套深度超过限制时，lua_to_json 返回错误标记而不是 panic
    #[tokio::test]
    async fn test_lua_to_json_recursion_limit() {
        // 创建深度嵌套的 Lua 表（超过默认的32层限制）
        let depth = 40;
        let mut code = String::new();
        for _ in 0..depth {
            code.push_str("{ nested = ");
        }
        code.push_str("1");
        for _ in 0..depth {
            code.push_str(" }");
        }

        // 使用默认引擎执行
        let result = engine()
            .execute(
                &Script::lua("t", &format!("return {}", code)),
                &default_ctx(),
            )
            .await
            .expect("执行不应失败");

        // 验证结果：超过限制的部分应该变成错误标记字符串
        // 而不是导致 panic 或无限递归
        let mut current = &result;
        let mut actual_depth = 0;
        while let Some(nested) = current.get("nested") {
            if nested.is_string() {
                // 遇到了错误标记字符串
                let err_msg = nested.as_str().unwrap();
                assert!(
                    err_msg.contains("Recursion limit exceeded"),
                    "超过递归限制时应返回错误标记，而不是: {}",
                    err_msg
                );
                break;
            }
            actual_depth += 1;
            current = nested;
        }

        assert!(
            actual_depth <= DEFAULT_MAX_RECURSION_DEPTH as usize,
            "嵌套深度不应超过限制 {}，实际深度: {}",
            DEFAULT_MAX_RECURSION_DEPTH,
            actual_depth
        );
    }

    /// Test: json_to_lua 超过深度限制时返回 RecursionLimitExceeded 错误
    ///
    /// 验证当 JSON 嵌套深度超过限制时，返回正确的错误类型
    #[tokio::test]
    async fn test_json_to_lua_recursion_limit() {
        // 创建深度嵌套的 JSON（超过默认的32层限制）
        let depth = 40;
        let mut val = json!(1);
        for _ in 0..depth {
            val = json!({ "nested": val });
        }

        let ctx = ScriptContext::new("test-agent").with_global("deep", val);

        // 执行应该失败，返回 RecursionLimitExceeded 错误
        let result = engine()
            .execute(&Script::lua("t", "return deep"), &ctx)
            .await;

        // 验证错误类型
        assert!(
            matches!(result, Err(ScriptError::RecursionLimitExceeded(_))),
            "超过递归限制时应返回 RecursionLimitExceeded 错误，而不是: {:?}",
            result
        );

        // 验证错误信息中包含正确的深度限制
        if let Err(ScriptError::RecursionLimitExceeded(max_depth)) = result {
            assert_eq!(
                max_depth, DEFAULT_MAX_RECURSION_DEPTH,
                "错误应报告正确的深度限制"
            );
        }
    }

    /// Test: 自定义递归深度限制生效
    ///
    /// 验证 LuaEngine::with_max_recursion_depth 设置的限制在实际执行中生效
    #[tokio::test]
    async fn test_lua_recursion_custom_depth_limit() {
        // 创建一个15层嵌套的 JSON
        let mut val = json!(1);
        for _ in 0..15 {
            val = json!({ "nested": val });
        }

        // 使用限制为10的引擎应该失败
        let limited_engine = LuaEngine::with_max_recursion_depth(10);
        let ctx = ScriptContext::new("test-agent").with_global("data", val.clone());
        let result = limited_engine
            .execute(&Script::lua("t", "return data"), &ctx)
            .await;

        assert!(
            matches!(result, Err(ScriptError::RecursionLimitExceeded(10))),
            "自定义深度限制 10 应该导致递归限制错误"
        );

        // 使用限制为20的引擎应该成功
        let larger_engine = LuaEngine::with_max_recursion_depth(20);
        let ctx = ScriptContext::new("test-agent").with_global("data", val);
        let result = larger_engine
            .execute(&Script::lua("t", "return data"), &ctx)
            .await;

        assert!(result.is_ok(), "深度限制 20 应该足够处理15层嵌套");
    }
}
