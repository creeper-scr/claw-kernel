//! Lua-Rust tools bridge for calling registered tools from scripts.

use std::sync::Arc;

use claw_tools::{
    registry::ToolRegistry,
    types::{PermissionSet, ToolContext},
};
use mlua::{Lua, Result as LuaResult, UserData, UserDataMethods};

use crate::bridge::conversion::{json_to_lua, lua_to_json};

/// Tools bridge exposing ToolRegistry to Lua scripts.
///
/// This bridge allows Lua scripts to:
/// - List available tools
/// - Check if a tool exists
/// - Call tools with JSON arguments
///
/// Each tool call includes the caller context for audit logging.
pub struct ToolsBridge {
    /// The tool registry to execute tools from.
    registry: Arc<ToolRegistry>,
    /// Context identifying the calling agent.
    caller_context: CallerContext,
}

/// Context identifying the caller for audit and permission purposes.
#[derive(Debug, Clone)]
pub struct CallerContext {
    /// Agent ID of the caller.
    pub agent_id: String,
    /// Permissions granted to the caller.
    pub permissions: PermissionSet,
}

impl CallerContext {
    /// Create a new caller context.
    pub fn new(agent_id: impl Into<String>, permissions: PermissionSet) -> Self {
        Self {
            agent_id: agent_id.into(),
            permissions,
        }
    }
}

/// Result of a tool call from Lua perspective.
#[derive(Debug, Clone)]
pub struct ToolCallResult {
    /// Whether the call was successful.
    pub success: bool,
    /// Output value (present on success).
    pub output: Option<serde_json::Value>,
    /// Error message (present on failure).
    pub error: Option<String>,
}

impl ToolsBridge {
    /// Create a new ToolsBridge with the given registry and caller context.
    pub fn new(registry: Arc<ToolRegistry>, caller_context: CallerContext) -> Self {
        Self {
            registry,
            caller_context,
        }
    }

    /// Call a tool by name with JSON arguments.
    ///
    /// Returns a table with `success`, `output`, and `error` fields.
    pub async fn call(&self, name: &str, args: serde_json::Value) -> ToolCallResult {
        let ctx = ToolContext::new(
            &self.caller_context.agent_id,
            self.caller_context.permissions.clone(),
        );

        match self.registry.execute(name, args, ctx).await {
            Ok(result) => {
                if result.success {
                    ToolCallResult {
                        success: true,
                        output: result.output,
                        error: None,
                    }
                } else {
                    ToolCallResult {
                        success: false,
                        output: None,
                        error: result.error.map(|e| e.message),
                    }
                }
            }
            Err(e) => ToolCallResult {
                success: false,
                output: None,
                error: Some(e.to_string()),
            },
        }
    }

    /// List all available tools.
    ///
    /// Returns a table of tool info with `name` and `description` fields.
    pub fn list(&self) -> Vec<ToolInfo> {
        self.registry
            .tool_names()
            .into_iter()
            .filter_map(|name| {
                self.registry.tool_meta(&name).map(|meta| ToolInfo {
                    name,
                    description: meta.schema.description,
                })
            })
            .collect()
    }

    /// Check if a tool exists.
    pub fn exists(&self, name: &str) -> bool {
        self.registry.tool_meta(name).is_some()
    }
}

/// Information about a tool for Lua.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

impl UserData for ToolInfo {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("name", |_, this, ()| Ok(this.name.clone()));
        methods.add_method("description", |_, this, ()| Ok(this.description.clone()));
    }
}

impl UserData for ToolCallResult {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        methods.add_method("success", |_, this, ()| Ok(this.success));
        methods.add_method("output", |lua, this, ()| {
            match &this.output {
                Some(v) => {
                    // Convert JSON value to Lua value
                    json_to_lua(lua, v, 0)
                }
                None => Ok(mlua::Value::Nil),
            }
        });
        methods.add_method("error", |_, this, ()| {
            Ok(this.error.clone().unwrap_or_default())
        });
    }
}

impl UserData for ToolsBridge {
    fn add_methods<'lua, M: UserDataMethods<'lua, Self>>(methods: &mut M) {
        // NOTE: `call` uses `add_method` with `block_on` because it runs inside
        // `spawn_blocking` context (see LuaEngine::eval/exec).
        // call(name, args) -> ToolCallResult
        methods.add_method(
            "call",
            |_lua, this, (name, args): (String, Option<mlua::Value>)| {
                // Note: Called from spawn_blocking context; use block_on for async operations.
                let json_args = match args {
                    Some(v) => lua_to_json(v),
                    None => serde_json::Value::Object(serde_json::Map::new()),
                };
                let result = tokio::runtime::Handle::current()
                    .block_on(this.call(&name, json_args));
                Ok(result)
            },
        );

        // list() -> table of ToolInfo
        methods.add_method("list", |lua, this, ()| {
            let tools = this.list();
            let table = lua.create_table()?;
            for (i, tool) in tools.into_iter().enumerate() {
                table.raw_set(i + 1, tool)?;
            }
            Ok(table)
        });

        // exists(name) -> bool
        methods.add_method("exists", |_, this, name: String| Ok(this.exists(&name)));
    }
}

/// Register the ToolsBridge as a global `tools` table in the Lua instance.
///
/// # Example in Lua:
/// ```lua
/// -- Check if a tool exists
/// if tools:exists("echo") then
///     -- Call the tool
///     local result = tools:call("echo", {message = "Hello"})
///     if result:success() then
///         print(result:output())
///     else
///         print("Error: " .. result:error())
///     end
/// end
///
/// -- List all tools
/// local available_tools = tools:list()
/// for _, tool in ipairs(available_tools) do
///     print(tool:name(), tool:description())
/// end
/// ```
pub fn register_tools(lua: &Lua, bridge: ToolsBridge) -> LuaResult<()> {
    lua.globals().set("tools", bridge)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use claw_tools::{
        traits::Tool,
        types::{PermissionSet, ToolContext, ToolResult, ToolSchema},
    };

    struct MockTool {
        name: String,
        description: String,
    }

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            &self.description
        }

        fn schema(&self) -> &ToolSchema {
            // This is a bit hacky for testing, but works
            static SCHEMA: std::sync::OnceLock<ToolSchema> = std::sync::OnceLock::new();
            SCHEMA.get_or_init(|| ToolSchema::new("mock", "Mock tool", serde_json::json!({})))
        }

        fn permissions(&self) -> &PermissionSet {
            static PERMS: std::sync::OnceLock<PermissionSet> = std::sync::OnceLock::new();
            PERMS.get_or_init(PermissionSet::minimal)
        }

        async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::ok(args, 0)
        }
    }

    fn create_test_registry() -> Arc<ToolRegistry> {
        let registry = Arc::new(ToolRegistry::new());
        registry
            .register(Box::new(MockTool {
                name: "echo".to_string(),
                description: "Echo tool".to_string(),
            }))
            .unwrap();
        registry
            .register(Box::new(MockTool {
                name: "reverse".to_string(),
                description: "Reverse text tool".to_string(),
            }))
            .unwrap();
        registry
    }

    #[test]
    fn test_tools_bridge_exists() {
        let registry = create_test_registry();
        let bridge = ToolsBridge::new(
            registry,
            CallerContext::new("agent-1", PermissionSet::minimal()),
        );

        assert!(bridge.exists("echo"));
        assert!(bridge.exists("reverse"));
        assert!(!bridge.exists("nonexistent"));
    }

    #[test]
    fn test_tools_bridge_list() {
        let registry = create_test_registry();
        let bridge = ToolsBridge::new(
            registry,
            CallerContext::new("agent-1", PermissionSet::minimal()),
        );

        let tools = bridge.list();
        assert_eq!(tools.len(), 2);

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"echo"));
        assert!(names.contains(&"reverse"));
    }

    #[tokio::test]
    async fn test_tools_bridge_call_success() {
        let registry = create_test_registry();
        let bridge = ToolsBridge::new(
            registry,
            CallerContext::new("agent-1", PermissionSet::minimal()),
        );

        let result = bridge
            .call("echo", serde_json::json!({"message": "hello"}))
            .await;

        assert!(result.success);
        assert!(result.error.is_none());
        assert!(result.output.is_some());
    }

    #[tokio::test]
    async fn test_tools_bridge_call_nonexistent() {
        let registry = create_test_registry();
        let bridge = ToolsBridge::new(
            registry,
            CallerContext::new("agent-1", PermissionSet::minimal()),
        );

        let result = bridge.call("nonexistent", serde_json::json!({})).await;

        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_json_lua_conversions() {
        // Test JSON to Lua conversion
        let lua = unsafe { Lua::unsafe_new() };

        let json = serde_json::json!({
            "string": "value",
            "number": 42,
            "float": 3.14,
            "bool": true,
            "null": null,
            "array": [1, 2, 3],
            "nested": {"key": "value"}
        });

        let lua_val = json_to_lua(&lua, &json, 0).unwrap();

        // Verify it's a table
        match lua_val {
            mlua::Value::Table(t) => {
                assert_eq!(t.get::<_, String>("string").unwrap(), "value");
                assert_eq!(t.get::<_, i64>("number").unwrap(), 42);
                assert_eq!(t.get::<_, bool>("bool").unwrap(), true);

                let arr: mlua::Table = t.get("array").unwrap();
                assert_eq!(arr.raw_len(), 3);
                assert_eq!(arr.get::<_, i64>(1).unwrap(), 1);
                assert_eq!(arr.get::<_, i64>(2).unwrap(), 2);
                assert_eq!(arr.get::<_, i64>(3).unwrap(), 3);
            }
            _ => panic!("Expected table"),
        }
    }

    #[test]
    fn test_lua_json_conversions() {
        let lua = unsafe { Lua::unsafe_new() };

        // Create a Lua table
        let table = lua.create_table().unwrap();
        table.set("name", "test").unwrap();
        table.set("value", 42).unwrap();

        let arr = lua.create_table().unwrap();
        arr.raw_set(1, "a").unwrap();
        arr.raw_set(2, "b").unwrap();
        table.set("items", arr).unwrap();

        let json = lua_to_json(mlua::Value::Table(table));

        match json {
            serde_json::Value::Object(map) => {
                assert_eq!(map["name"], "test");
                assert_eq!(map["value"], 42);
                assert!(map["items"].is_array());
            }
            _ => panic!("Expected object"),
        }
    }

    #[test]
    fn test_tool_info_userdata() {
        let info = ToolInfo {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
        };

        let lua = unsafe { Lua::unsafe_new() };
        lua.globals().set("info", info).unwrap();

        let result: String = lua.load("return info:name()").eval().unwrap();
        assert_eq!(result, "test_tool");

        let result: String = lua.load("return info:description()").eval().unwrap();
        assert_eq!(result, "A test tool");
    }
}
