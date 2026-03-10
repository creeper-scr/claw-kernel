//! [`LuaToolCompiler`] — bridges claw-script's Lua engine into claw-tools'
//! hot-reload pipeline via the [`ScriptToolCompiler`] trait.
//!
//! # Lua tool script convention
//!
//! A script file loaded by [`LuaToolCompiler`] must `return` a Lua table that
//! follows this shape:
//!
//! ```lua
//! return {
//!     name        = "my_tool",          -- optional; falls back to file stem
//!     description = "Does X given Y",   -- optional; falls back to "Script tool"
//!     schema      = {                   -- optional; falls back to empty object
//!         type = "object",
//!         properties = {
//!             input = { type = "string", description = "The input" }
//!         },
//!         required = {"input"}
//!     },
//!     execute = function(args)          -- REQUIRED
//!         return { result = "processed " .. args.input }
//!     end
//! }
//! ```
//!
//! The `execute` field is mandatory. Compilation fails with a
//! [`LoadError::ParseError`] when it is absent.

use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use claw_tools::{
    traits::{ScriptToolCompiler, Tool},
    types::{
        FsPermissions, NetworkPermissions, PermissionSet, SubprocessPolicy, ToolContext, ToolError,
        ToolResult, ToolSchema,
    },
};
use claw_tools::error::LoadError;

use crate::{
    traits::ScriptEngine,
    types::{Script, ScriptContext},
    LuaEngine,
};

// ─── LuaToolCompiler ────────────────────────────────────────────────────────

/// Compiles Lua script files into [`Tool`] implementations.
///
/// Implements [`ScriptToolCompiler`] so it can be injected into
/// [`claw_tools::hot_reload::HotReloadProcessor`]:
///
/// ```rust,ignore
/// use claw_script::LuaToolCompiler;
/// use claw_tools::hot_reload::HotReloadProcessor;
/// use claw_tools::registry::ToolRegistry;
/// use std::sync::Arc;
///
/// let registry = Arc::new(ToolRegistry::new());
/// let config = Default::default();
/// let processor = HotReloadProcessor::new(registry, config)
///     .with_compiler(LuaToolCompiler::arc());
/// ```
pub struct LuaToolCompiler;

impl LuaToolCompiler {
    /// Create a new [`LuaToolCompiler`].
    pub fn new() -> Self {
        Self
    }

    /// Create a new [`LuaToolCompiler`] wrapped in `Arc<dyn ScriptToolCompiler>`.
    ///
    /// Convenience method for use with
    /// [`HotReloadProcessor::with_compiler`](claw_tools::hot_reload::HotReloadProcessor::with_compiler).
    #[allow(clippy::new_ret_no_self)]
    pub fn arc() -> Arc<dyn ScriptToolCompiler> {
        Arc::new(Self)
    }
}

impl Default for LuaToolCompiler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ScriptToolCompiler for LuaToolCompiler {
    fn supports_extension(&self, ext: &str) -> bool {
        ext == "lua"
    }

    async fn compile(&self, path: &Path, content: &str) -> Result<Arc<dyn Tool>, LoadError> {
        // Derive a default tool name from the file stem.
        let file_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| LoadError::ParseError("invalid file name".to_string()))?
            .to_string();

        let engine = LuaEngine::new();
        let content_owned = content.to_string();

        // ── Step 1: syntax validation ────────────────────────────────────────
        let validate_script = Script::lua(&file_stem, &content_owned);
        engine
            .validate(&validate_script)
            .map_err(|e| LoadError::ParseError(format!("syntax error: {}", e)))?;

        // ── Step 2: metadata extraction ──────────────────────────────────────
        // Wrap the script body in an IIFE and return only the metadata fields.
        // This avoids storing Lua closures across VM lifetimes.
        let meta_wrapper = format!(
            "local __t = (function()\n{}\nend)()\n\
             if type(__t) ~= \"table\" then\n\
               error(\"tool script must return a table, got \" .. type(__t))\n\
             end\n\
             if type(__t.execute) ~= \"function\" then\n\
               error(\"tool script table must have an 'execute' function field\")\n\
             end\n\
             return {{ name = __t.name, description = __t.description, schema = __t.schema, permissions = __t.permissions }}",
            content_owned
        );
        let meta_script = Script::lua(&file_stem, &meta_wrapper);
        let ctx = ScriptContext::new(format!("compiler-{}", file_stem))
            .with_timeout(Duration::from_secs(5));

        let meta = engine
            .execute(&meta_script, &ctx)
            .await
            .map_err(|e| LoadError::ParseError(format!("metadata extraction failed: {}", e)))?;

        // ── Step 3: build ToolSchema ─────────────────────────────────────────
        let name = meta
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&file_stem)
            .to_string();

        let description = meta
            .get("description")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("Script tool")
            .to_string();

        let schema_value = meta
            .get("schema")
            .cloned()
            .filter(|v| v.is_object())
            .unwrap_or_else(|| serde_json::json!({ "type": "object", "properties": {} }));

        let schema = ToolSchema::new(&name, &description, schema_value);

        let permissions = parse_permissions(meta.get("permissions"));

        Ok(Arc::new(ScriptTool {
            tool_name: name,
            tool_description: description,
            schema,
            script_content: content_owned,
            permissions,
        }))
    }
}

// ─── Permission parsing ───────────────────────────────────────────────────────

/// Parse a `PermissionSet` from the optional `permissions` table returned by a
/// Lua tool script.
///
/// The expected Lua table shape is:
///
/// ```lua
/// permissions = {
///     fs_read    = { "/tmp/**", "/home/user/data/**" },  -- optional
///     fs_write   = { "/tmp/out/**" },                     -- optional
///     network    = { "api.example.com", "cdn.example.io" }, -- optional
///     subprocess = false,                                 -- optional bool
/// }
/// ```
///
/// Any absent or `nil` key falls back to an empty / denied value.
/// If `value` itself is `None` or not an object the function returns
/// [`PermissionSet::minimal()`].
fn parse_permissions(value: Option<&serde_json::Value>) -> PermissionSet {
    let Some(obj) = value.and_then(|v| v.as_object()) else {
        return PermissionSet::minimal();
    };

    let string_list = |key: &str| -> HashSet<String> {
        obj.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };

    let fs_read = string_list("fs_read");
    let fs_write = string_list("fs_write");
    let network_domains = string_list("network");
    let subprocess = obj
        .get("subprocess")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    PermissionSet {
        filesystem: FsPermissions {
            read_paths: fs_read,
            write_paths: fs_write,
        },
        network: if network_domains.is_empty() {
            NetworkPermissions::none()
        } else {
            NetworkPermissions::allow(network_domains)
        },
        subprocess: if subprocess {
            SubprocessPolicy::Allowed
        } else {
            SubprocessPolicy::Denied
        },
    }
}

// ─── ScriptTool ──────────────────────────────────────────────────────────────

/// A [`Tool`] that executes a Lua script on every invocation.
///
/// Each call to [`Tool::execute`] spins up a fresh Lua VM, re-evaluates the
/// script (to obtain the tool table), and calls `table.execute(args)`.
/// This is intentionally stateless and safe for concurrent use.
struct ScriptTool {
    tool_name: String,
    tool_description: String,
    schema: ToolSchema,
    script_content: String,
    permissions: PermissionSet,
}

#[async_trait]
impl Tool for ScriptTool {
    fn name(&self) -> &str {
        &self.tool_name
    }

    fn description(&self) -> &str {
        &self.tool_description
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    fn permissions(&self) -> &PermissionSet {
        &self.permissions
    }

    async fn execute(&self, args: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let engine = LuaEngine::new();

        // Wrap the script so we call `table.execute(args)` and return the result.
        let exec_wrapper = format!(
            "local __t = (function()\n{}\nend)()\n\
             return __t.execute(__claw_args__)",
            self.script_content
        );

        let script = Script::lua(&self.tool_name, &exec_wrapper);
        let script_ctx = ScriptContext::new(ctx.agent_id.clone())
            .with_global("__claw_args__", args)
            .with_timeout(self.timeout());

        match engine.execute(&script, &script_ctx).await {
            Ok(output) => ToolResult::ok(output, 0),
            Err(e) => ToolResult::err(ToolError::internal(e.to_string()), 0),
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn compiler() -> LuaToolCompiler {
        LuaToolCompiler::new()
    }

    // ── extension support ──

    #[test]
    fn test_supports_lua_extension() {
        assert!(compiler().supports_extension("lua"));
    }

    #[test]
    fn test_rejects_non_lua_extension() {
        assert!(!compiler().supports_extension("js"));
        assert!(!compiler().supports_extension("py"));
        assert!(!compiler().supports_extension(""));
    }

    // ── compile: happy-path ──

    #[tokio::test]
    async fn test_compile_basic_tool() {
        let script = r#"
return {
    name        = "greet",
    description = "Greets someone",
    schema = {
        type = "object",
        properties = { name = { type = "string" } },
        required = {"name"}
    },
    execute = function(args)
        return { greeting = "Hello, " .. args.name }
    end
}
"#;
        let tool = compiler()
            .compile(Path::new("/tools/greet.lua"), script)
            .await
            .expect("compile must succeed");

        assert_eq!(tool.name(), "greet");
        assert_eq!(tool.description(), "Greets someone");
    }

    #[tokio::test]
    async fn test_compile_uses_file_stem_when_name_absent() {
        let script = r#"
return {
    description = "No explicit name",
    execute = function(args) return {} end
}
"#;
        let tool = compiler()
            .compile(Path::new("/tools/my_util.lua"), script)
            .await
            .expect("compile must succeed");

        assert_eq!(tool.name(), "my_util");
    }

    #[tokio::test]
    async fn test_compile_uses_default_description() {
        let script = "return { execute = function(args) return {} end }";
        let tool = compiler()
            .compile(Path::new("/tools/t.lua"), script)
            .await
            .expect("compile must succeed");

        assert_eq!(tool.description(), "Script tool");
    }

    // ── compile: error cases ──

    #[tokio::test]
    async fn test_compile_syntax_error() {
        let result = compiler()
            .compile(Path::new("/tools/bad.lua"), "@@@ not lua")
            .await;
        assert!(matches!(result, Err(LoadError::ParseError(_))));
    }

    #[tokio::test]
    async fn test_compile_missing_execute_field() {
        let script = "return { name = \"no_exec\" }";
        let result = compiler()
            .compile(Path::new("/tools/no_exec.lua"), script)
            .await;
        assert!(matches!(result, Err(LoadError::ParseError(_))));
    }

    #[tokio::test]
    async fn test_compile_non_table_return() {
        let script = "return 42";
        let result = compiler()
            .compile(Path::new("/tools/bad.lua"), script)
            .await;
        assert!(matches!(result, Err(LoadError::ParseError(_))));
    }

    // ── ScriptTool::execute ──

    #[tokio::test]
    async fn test_script_tool_execute() {
        let script = r#"
return {
    name = "add",
    description = "Adds two numbers",
    schema = {
        type = "object",
        properties = {
            a = { type = "number" },
            b = { type = "number" }
        },
        required = {"a", "b"}
    },
    execute = function(args)
        return { sum = args.a + args.b }
    end
}
"#;
        let tool = compiler()
            .compile(Path::new("/tools/add.lua"), script)
            .await
            .expect("compile must succeed");

        let ctx = claw_tools::types::ToolContext::new("test-agent", PermissionSet::minimal());
        let args = serde_json::json!({ "a": 3, "b": 4 });
        let result = tool.execute(args, &ctx).await;

        assert!(result.success, "execute must succeed");
        assert_eq!(result.output.unwrap()["sum"], serde_json::json!(7));
    }

    // ── permissions ──

    #[tokio::test]
    async fn test_compile_parses_fs_read_permissions() {
        let script = r#"
return {
    name = "reader",
    permissions = {
        fs_read = { "/tmp/**", "/data/shared/**" },
    },
    execute = function(args) return {} end
}
"#;
        let tool = compiler()
            .compile(Path::new("/tools/reader.lua"), script)
            .await
            .expect("compile must succeed");

        let perms = tool.permissions();
        assert!(perms.filesystem.read_paths.contains("/tmp/**"));
        assert!(perms.filesystem.read_paths.contains("/data/shared/**"));
        assert!(perms.filesystem.write_paths.is_empty());
        assert!(perms.network.allowed_domains.is_empty());
    }

    #[tokio::test]
    async fn test_compile_parses_network_permissions() {
        let script = r#"
return {
    name = "fetcher",
    permissions = {
        network = { "api.example.com", "cdn.example.io" },
    },
    execute = function(args) return {} end
}
"#;
        let tool = compiler()
            .compile(Path::new("/tools/fetcher.lua"), script)
            .await
            .expect("compile must succeed");

        let perms = tool.permissions();
        assert!(perms.network.allowed_domains.contains("api.example.com"));
        assert!(perms.network.allowed_domains.contains("cdn.example.io"));
        assert!(perms.filesystem.read_paths.is_empty());
    }

    #[tokio::test]
    async fn test_compile_parses_subprocess_permission() {
        let script = r#"
return {
    name = "runner",
    permissions = { subprocess = true },
    execute = function(args) return {} end
}
"#;
        let tool = compiler()
            .compile(Path::new("/tools/runner.lua"), script)
            .await
            .expect("compile must succeed");

        assert_eq!(
            tool.permissions().subprocess,
            claw_tools::types::SubprocessPolicy::Allowed
        );
    }

    #[tokio::test]
    async fn test_compile_no_permissions_falls_back_to_minimal() {
        let script = "return { execute = function(args) return {} end }";
        let tool = compiler()
            .compile(Path::new("/tools/t.lua"), script)
            .await
            .expect("compile must succeed");

        let perms = tool.permissions();
        assert!(perms.filesystem.read_paths.is_empty());
        assert!(perms.filesystem.write_paths.is_empty());
        assert!(perms.network.allowed_domains.is_empty());
        assert_eq!(perms.subprocess, claw_tools::types::SubprocessPolicy::Denied);
    }

    #[test]
    fn test_parse_permissions_none() {
        let perms = parse_permissions(None);
        assert!(perms.filesystem.read_paths.is_empty());
        assert_eq!(perms.subprocess, claw_tools::types::SubprocessPolicy::Denied);
    }

    #[test]
    fn test_parse_permissions_full() {
        let value = serde_json::json!({
            "fs_read": ["/tmp/**"],
            "fs_write": ["/out/**"],
            "network": ["example.com"],
            "subprocess": true,
        });
        let perms = parse_permissions(Some(&value));
        assert!(perms.filesystem.read_paths.contains("/tmp/**"));
        assert!(perms.filesystem.write_paths.contains("/out/**"));
        assert!(perms.network.allowed_domains.contains("example.com"));
        assert_eq!(perms.subprocess, claw_tools::types::SubprocessPolicy::Allowed);
    }

    #[tokio::test]
    async fn test_script_tool_execute_runtime_error() {
        let script = r#"
return {
    name = "fail",
    execute = function(args)
        error("intentional failure")
    end
}
"#;
        let tool = compiler()
            .compile(Path::new("/tools/fail.lua"), script)
            .await
            .expect("compile must succeed");

        let ctx = claw_tools::types::ToolContext::new("test-agent", PermissionSet::minimal());
        let result = tool.execute(serde_json::json!({}), &ctx).await;

        assert!(!result.success, "execute must fail");
        let err = result.error.unwrap();
        assert!(
            err.message.contains("intentional failure"),
            "error message should propagate: {}",
            err.message
        );
    }
}
