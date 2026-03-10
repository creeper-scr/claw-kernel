//! Shared JSON ↔ Lua conversion utilities for bridge modules.
//!
//! Note: these functions are called from `spawn_blocking` context.

use mlua::Lua;

/// Convert a [`serde_json::Value`] to a [`mlua::Value`].
///
/// Recursion depth is capped at 32 to prevent stack overflows.
pub(crate) fn json_to_lua<'lua>(
    lua: &'lua Lua,
    val: &serde_json::Value,
    depth: u32,
) -> mlua::Result<mlua::Value<'lua>> {
    if depth > 32 {
        return Ok(mlua::Value::Nil);
    }

    let lval = match val {
        serde_json::Value::Null => mlua::Value::Nil,
        serde_json::Value::Bool(b) => mlua::Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                mlua::Value::Integer(i)
            } else {
                mlua::Value::Number(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => mlua::Value::String(lua.create_string(s.as_bytes())?),
        serde_json::Value::Array(arr) => {
            let table = lua.create_table()?;
            for (i, elem) in arr.iter().enumerate() {
                table.raw_set(i + 1, json_to_lua(lua, elem, depth + 1)?)?;
            }
            mlua::Value::Table(table)
        }
        serde_json::Value::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.raw_set(k.as_str(), json_to_lua(lua, v, depth + 1)?)?;
            }
            mlua::Value::Table(table)
        }
    };

    Ok(lval)
}

/// Convert a [`mlua::Value`] to a [`serde_json::Value`].
pub(crate) fn lua_to_json(val: mlua::Value<'_>) -> serde_json::Value {
    match val {
        mlua::Value::Nil => serde_json::Value::Null,
        mlua::Value::Boolean(b) => serde_json::Value::Bool(b),
        mlua::Value::Integer(i) => serde_json::json!(i),
        mlua::Value::Number(f) => serde_json::json!(f),
        mlua::Value::String(s) => serde_json::Value::String(s.to_str().unwrap_or("").to_string()),
        mlua::Value::Table(t) => {
            // Try to detect if it's an array
            let len = t.raw_len();
            if len > 0 {
                let arr: Vec<serde_json::Value> = (1..=(len as i64))
                    .filter_map(|i| t.raw_get::<i64, mlua::Value>(i).ok())
                    .map(lua_to_json)
                    .collect();
                if arr.len() == len {
                    return serde_json::Value::Array(arr);
                }
            }
            // Otherwise treat as object
            let mut map = serde_json::Map::new();
            for (k, v) in t.pairs::<mlua::Value, mlua::Value>().flatten() {
                let key = match k {
                    mlua::Value::String(s) => s.to_str().unwrap_or("").to_string(),
                    mlua::Value::Integer(i) => i.to_string(),
                    _ => continue,
                };
                map.insert(key, lua_to_json(v));
            }
            serde_json::Value::Object(map)
        }
        _ => serde_json::Value::Null,
    }
}
