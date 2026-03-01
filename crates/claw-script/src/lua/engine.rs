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
fn lua_to_json(val: LuaValue) -> JsonValue {
    match val {
        LuaValue::Nil => JsonValue::Null,
        LuaValue::Boolean(b) => json!(b),
        LuaValue::Integer(i) => json!(i),
        LuaValue::Number(f) => json!(f),
        LuaValue::String(s) => json!(s.to_str().unwrap_or("")),
        _ => JsonValue::Null,
    }
}

/// Convert a `serde_json::Value` into a mlua `Value`.
fn json_to_lua<'lua>(lua: &'lua Lua, val: &JsonValue) -> mlua::Result<LuaValue<'lua>> {
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
        // Tables / arrays are not injected as globals in this implementation
        _ => LuaValue::Nil,
    };
    Ok(lval)
}

#[async_trait]
impl ScriptEngine for LuaEngine {
    fn engine_type(&self) -> &str {
        "lua"
    }

    async fn execute(&self, script: &Script, ctx: &ScriptContext) -> Result<ScriptValue, ScriptError> {
        let source = script.source.clone();
        let agent_id = ctx.agent_id.clone();
        let globals_map = ctx.globals.clone();

        let result = tokio::task::spawn_blocking(move || -> Result<ScriptValue, ScriptError> {
            let lua = Lua::new();

            // Inject agent_id global
            lua.globals()
                .set("agent_id", agent_id)
                .map_err(|e| ScriptError::Runtime(e.to_string()))?;

            // Inject caller-supplied globals
            for (key, val) in &globals_map {
                let lua_val = json_to_lua(&lua, val)
                    .map_err(|e| ScriptError::Runtime(e.to_string()))?;
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
        })
        .await
        .map_err(|e| ScriptError::Runtime(e.to_string()))??;

        Ok(result)
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
}
