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

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use claw_tools::{
    traits::{ScriptToolCompiler, Tool},
    types::{PermissionSet, ToolContext, ToolError, ToolResult, ToolSchema},
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
             return {{ name = __t.name, description = __t.description, schema = __t.schema }}",
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

        Ok(Arc::new(ScriptTool {
            tool_name: name,
            tool_description: description,
            schema,
            script_content: content_owned,
            permissions: PermissionSet::minimal(),
        }))
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
