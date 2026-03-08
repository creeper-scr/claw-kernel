//! Dirs bridge — exposes platform directories to Lua scripts.

use mlua::{Lua, Result as LuaResult, UserData, UserDataMethods};

/// Dirs bridge exposing platform directories to Lua scripts.
///
/// Registered as the global `dirs` table.
///
/// # Example in Lua:
/// ```lua
/// local cfg = dirs:config_dir()
/// local data = dirs:data_dir()
/// local cache = dirs:cache_dir()
/// local tools = dirs:tools_dir()
/// ```
pub struct DirsBridge;

impl UserData for DirsBridge {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("config_dir", |_, _, ()| {
            Ok(claw_pal::dirs::config_dir()
                .map(|p| p.to_string_lossy().to_string()))
        });

        methods.add_method("data_dir", |_, _, ()| {
            Ok(claw_pal::dirs::data_dir()
                .map(|p| p.to_string_lossy().to_string()))
        });

        methods.add_method("cache_dir", |_, _, ()| {
            Ok(claw_pal::dirs::cache_dir()
                .map(|p| p.to_string_lossy().to_string()))
        });

        methods.add_method("tools_dir", |_, _, ()| {
            Ok(claw_pal::dirs::tools_dir()
                .map(|p| p.to_string_lossy().to_string()))
        });

        methods.add_method("scripts_dir", |_, _, ()| {
            Ok(claw_pal::dirs::scripts_dir()
                .map(|p| p.to_string_lossy().to_string()))
        });

        methods.add_method("logs_dir", |_, _, ()| {
            Ok(claw_pal::dirs::logs_dir()
                .map(|p| p.to_string_lossy().to_string()))
        });
    }
}

/// Register the DirsBridge as a global `dirs` table in the Lua instance.
pub fn register_dirs(lua: &Lua, bridge: DirsBridge) -> LuaResult<()> {
    lua.globals().set("dirs", bridge)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dirs_bridge_register() {
        let lua = Lua::new();
        register_dirs(&lua, DirsBridge).unwrap();
        // Should be accessible as global
        let result: bool = lua.load("return dirs ~= nil").eval().unwrap();
        assert!(result);
    }

    #[test]
    fn test_dirs_bridge_config_dir_returns_string_or_nil() {
        let lua = Lua::new();
        register_dirs(&lua, DirsBridge).unwrap();
        // Should return either a string or nil, never error
        let result: bool = lua
            .load("local d = dirs:config_dir(); return d == nil or type(d) == 'string'")
            .eval()
            .unwrap();
        assert!(result);
    }

    #[test]
    fn test_dirs_bridge_data_dir_returns_string_or_nil() {
        let lua = Lua::new();
        register_dirs(&lua, DirsBridge).unwrap();
        let result: bool = lua
            .load("local d = dirs:data_dir(); return d == nil or type(d) == 'string'")
            .eval()
            .unwrap();
        assert!(result);
    }

    #[test]
    fn test_dirs_bridge_cache_dir_returns_string_or_nil() {
        let lua = Lua::new();
        register_dirs(&lua, DirsBridge).unwrap();
        let result: bool = lua
            .load("local d = dirs:cache_dir(); return d == nil or type(d) == 'string'")
            .eval()
            .unwrap();
        assert!(result);
    }

    #[test]
    fn test_dirs_bridge_tools_dir_returns_string_or_nil() {
        let lua = Lua::new();
        register_dirs(&lua, DirsBridge).unwrap();
        let result: bool = lua
            .load("local d = dirs:tools_dir(); return d == nil or type(d) == 'string'")
            .eval()
            .unwrap();
        assert!(result);
    }

    #[test]
    fn test_dirs_bridge_all_methods_accessible() {
        let lua = Lua::new();
        register_dirs(&lua, DirsBridge).unwrap();
        // All methods should be callable without error
        let result: bool = lua
            .load(r#"
                local ok = true
                local _ = dirs:config_dir()
                local _ = dirs:data_dir()
                local _ = dirs:cache_dir()
                local _ = dirs:tools_dir()
                local _ = dirs:scripts_dir()
                local _ = dirs:logs_dir()
                return true
            "#)
            .eval()
            .unwrap();
        assert!(result);
    }
}
