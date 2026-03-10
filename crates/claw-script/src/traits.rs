use std::path::Path;

use async_trait::async_trait;

use crate::{
    error::ScriptError,
    types::{ModuleHandle, Script, ScriptContext, ScriptValue},
};

/// Core trait for embedded script engines.
#[async_trait]
pub trait ScriptEngine: Send + Sync {
    /// Name of this engine (e.g., "lua").
    fn engine_type(&self) -> &str;

    /// Execute a script and return the last expression value.
    async fn execute(
        &self,
        script: &Script,
        ctx: &ScriptContext,
    ) -> Result<ScriptValue, ScriptError>;

    /// Call a named function that was defined in a previously loaded module.
    ///
    /// The `module` handle is obtained via [`load_module`].  `fn_name` must
    /// refer to a top-level function (or table path, engine-specific) exported
    /// by that module.  `args` are passed as positional arguments.
    ///
    /// Returns the function's return value as a JSON value.
    async fn call(
        &self,
        module: &ModuleHandle,
        fn_name: &str,
        args: Vec<serde_json::Value>,
        ctx: &ScriptContext,
    ) -> Result<ScriptValue, ScriptError>;

    /// Load a module from disk and return an opaque handle.
    ///
    /// The file is read, syntax-validated, and returned as a `ModuleHandle`.
    /// Actual execution / function registration happens lazily inside `call()`.
    ///
    /// # Errors
    /// - [`ScriptError::ModuleNotFound`] — path does not exist or cannot be read.
    /// - [`ScriptError::Compile`] — source has syntax errors.
    async fn load_module(&self, path: &Path) -> Result<ModuleHandle, ScriptError>;

    /// Check if a script compiles (no execution).
    fn validate(&self, script: &Script) -> Result<(), ScriptError>;
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "engine-lua")]
    #[tokio::test]
    async fn test_script_engine_trait_object() {
        use crate::{types::Script, LuaEngine};

        // Verify that LuaEngine can be used as a Box<dyn ScriptEngine>
        let engine: Box<dyn ScriptEngine> = Box::new(LuaEngine::new());
        assert_eq!(engine.engine_type(), "lua");

        let script = Script::lua("test", "return 1 + 1");
        let ctx = crate::types::ScriptContext::new("agent-test");
        let result = engine.execute(&script, &ctx).await.unwrap();
        assert_eq!(result, serde_json::json!(2));
    }
}
