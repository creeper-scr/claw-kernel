use async_trait::async_trait;
use mlua::{Lua, Value as LuaValue};
use serde_json::{json, Value as JsonValue};

use crate::{
    error::{CompileError, ScriptError},
    traits::ScriptEngine,
    types::{Script, ScriptContext, ScriptValue},
};

/// Lua script engine backed by mlua (Lua 5.4).
///
/// Each execution creates a fresh Lua instance to guarantee isolation and
/// thread safety. Calls to mlua's synchronous API are dispatched via
/// `tokio::task::spawn_blocking` so the async executor is never blocked.
pub struct LuaEngine;

/// Convert a mlua `Value` into a `serde_json::Value`.
///
/// Tables are introspected: if `raw_len() > 0` and all 1..=n keys are present
/// the table is treated as a JSON array, otherwise as a JSON object.
fn lua_to_json(val: LuaValue) -> JsonValue {
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
                    .map(lua_to_json)
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
                map.insert(key, lua_to_json(v));
            }
            JsonValue::Object(map)
        }
        _ => JsonValue::Null,
    }
}

/// Convert a `serde_json::Value` into a mlua `Value`.
///
/// Recursion is depth-limited to 32 levels to prevent stack overflows.
fn json_to_lua<'lua>(lua: &'lua Lua, val: &JsonValue, depth: u32) -> mlua::Result<LuaValue<'lua>> {
    if depth > 32 {
        return Ok(LuaValue::Nil);
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
        JsonValue::String(s) => LuaValue::String(lua.create_string(s.as_bytes())?),
        JsonValue::Array(arr) => {
            let table = lua.create_table()?;
            for (i, elem) in arr.iter().enumerate() {
                table.raw_set(i + 1, json_to_lua(lua, elem, depth + 1)?)?;
            }
            LuaValue::Table(table)
        }
        JsonValue::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.raw_set(k.as_str(), json_to_lua(lua, v, depth + 1)?)?;
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

        let task = tokio::task::spawn_blocking(move || -> Result<ScriptValue, ScriptError> {
            let lua = Lua::new();

            // Inject agent_id global
            lua.globals()
                .set("agent_id", agent_id)
                .map_err(|e| ScriptError::Runtime(e.to_string()))?;

            // Inject caller-supplied globals
            for (key, val) in &globals_map {
                let lua_val =
                    json_to_lua(&lua, val, 0).map_err(|e| ScriptError::Runtime(e.to_string()))?;
                lua.globals()
                    .set(key.as_str(), lua_val)
                    .map_err(|e| ScriptError::Runtime(e.to_string()))?;
            }

            // Load and evaluate the script
            let chunk = lua.load(&source);
            let lua_result: LuaValue = chunk
                .eval()
                .map_err(|e| ScriptError::Runtime(e.to_string()))?;

            Ok(lua_to_json(lua_result))
        });

        match tokio::time::timeout(timeout_dur, task).await {
            Ok(join_result) => join_result.map_err(|e| ScriptError::Runtime(e.to_string()))?,
            Err(_elapsed) => Err(ScriptError::Timeout),
        }
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
        LuaEngine
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
        assert!(
            matches!(result, Err(ScriptError::Timeout)),
            "expected Timeout, got: {result:?}"
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
}
