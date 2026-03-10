use async_trait::async_trait;
use deno_core::v8;
use serde_json::Value as JsonValue;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::{
    error::{CompileError, ScriptError},
    traits::ScriptEngine,
    types::{EngineType, Script, ScriptContext, ScriptValue},
};

use super::bridge::BridgeState;
use super::transpile::{transpile_typescript, is_likely_typescript};

/// Default maximum execution time for V8 scripts.
const DEFAULT_TIMEOUT_MS: u64 = 30000;

/// Default heap limit for V8 isolate (in MB).
const DEFAULT_HEAP_LIMIT_MB: usize = 128;

/// Options for configuring the V8 engine.
#[derive(Clone, Debug)]
pub struct V8EngineOptions {
    /// Script execution timeout (default: 30s).
    pub timeout: Duration,
    /// V8 heap limit in MB (default: 128MB).
    pub heap_limit_mb: usize,
    /// Enable TypeScript support (default: true).
    /// When true, TypeScript scripts will be transpiled to JavaScript before execution.
    pub typescript: bool,
    /// Maximum recursion depth for JSON conversion (default: 32).
    pub max_recursion_depth: u32,
}

impl Default for V8EngineOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            heap_limit_mb: DEFAULT_HEAP_LIMIT_MB,
            typescript: true,
            max_recursion_depth: 32,
        }
    }
}

/// V8 script engine backed by deno_core.
///
/// Each execution creates a fresh V8 isolate to guarantee isolation and
/// security. The engine supports both JavaScript and TypeScript.
pub struct V8Engine {
    options: V8EngineOptions,
}

impl V8Engine {
    /// Create a new V8Engine with default configuration.
    pub fn new() -> Self {
        Self {
            options: V8EngineOptions::default(),
        }
    }

    /// Create a new V8Engine with custom options.
    pub fn with_options(options: V8EngineOptions) -> Self {
        Self { options }
    }

    /// Convert a serde_json::Value to a v8::Local<v8::Value>.
    fn json_to_v8<'a>(
        &self,
        scope: &mut v8::HandleScope<'a>,
        value: &JsonValue,
        depth: u32,
    ) -> Result<v8::Local<'a, v8::Value>, ScriptError> {
        if depth > self.options.max_recursion_depth {
            return Err(ScriptError::RecursionLimitExceeded(
                self.options.max_recursion_depth,
            ));
        }

        match value {
            JsonValue::Null => Ok(v8::null(scope).into()),
            JsonValue::Bool(b) => Ok(v8::Boolean::new(scope, *b).into()),
            JsonValue::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(v8::Number::new(scope, i as f64).into())
                } else {
                    Ok(v8::Number::new(scope, n.as_f64().unwrap_or(0.0)).into())
                }
            }
            JsonValue::String(s) => Ok(v8::String::new(scope, s)
                .map(|s| s.into())
                .unwrap_or_else(|| v8::undefined(scope).into())),
            JsonValue::Array(arr) => {
                let array = v8::Array::new(scope, arr.len() as i32);
                for (i, elem) in arr.iter().enumerate() {
                    let v8_elem = self.json_to_v8(scope, elem, depth + 1)?;
                    array.set_index(scope, i as u32, v8_elem);
                }
                Ok(array.into())
            }
            JsonValue::Object(map) => {
                let obj = v8::Object::new(scope);
                for (key, val) in map {
                    let v8_key = v8::String::new(scope, key).unwrap();
                    let v8_val = self.json_to_v8(scope, val, depth + 1)?;
                    obj.set(scope, v8_key.into(), v8_val);
                }
                Ok(obj.into())
            }
        }
    }

    /// Convert a v8::Local<v8::Value> to a serde_json::Value.
    fn v8_to_json<'a>(
        &self,
        scope: &mut v8::HandleScope<'a>,
        value: v8::Local<'a, v8::Value>,
        depth: u32,
    ) -> JsonValue {
        if depth > self.options.max_recursion_depth {
            return JsonValue::String(format!(
                "[Error: Recursion limit exceeded (max {} levels)]",
                self.options.max_recursion_depth
            ));
        }

        if value.is_null() {
            JsonValue::Null
        } else if value.is_undefined() {
            JsonValue::Null
        } else if value.is_boolean() {
            JsonValue::Bool(value.is_true())
        } else if value.is_number() {
            let num = value.number_value(scope).unwrap_or(0.0);
            if num.fract() == 0.0 && num >= i64::MIN as f64 && num <= i64::MAX as f64 {
                JsonValue::Number(serde_json::Number::from(num as i64))
            } else {
                match serde_json::Number::from_f64(num) {
                    Some(n) => JsonValue::Number(n),
                    None => JsonValue::Number(0.into()),
                }
            }
        } else if value.is_string() {
            JsonValue::String(
                value
                    .to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_default(),
            )
        } else if value.is_array() {
            let array = v8::Local::<v8::Array>::try_from(value).unwrap();
            let len = array.length();
            let mut vec = Vec::with_capacity(len as usize);
            for i in 0..len {
                let elem = array.get_index(scope, i).unwrap_or_else(|| v8::undefined(scope).into());
                vec.push(self.v8_to_json(scope, elem, depth + 1));
            }
            JsonValue::Array(vec)
        } else if value.is_object() {
            let obj = v8::Local::<v8::Object>::try_from(value).unwrap();
            let mut map = serde_json::Map::new();

            if let Some(props) = obj.get_own_property_names(scope, Default::default()) {
                let len = props.length();
                for i in 0..len {
                    if let Some(key) = props.get_index(scope, i) {
                        let key_str = key
                            .to_string(scope)
                            .map(|s| s.to_rust_string_lossy(scope))
                            .unwrap_or_default();
                        let val = obj.get(scope, key).unwrap_or_else(|| v8::undefined(scope).into());
                        map.insert(key_str, self.v8_to_json(scope, val, depth + 1));
                    }
                }
            }
            JsonValue::Object(map)
        } else {
            // Functions, Symbols, etc. -> null
            JsonValue::Null
        }
    }
}

impl Default for V8Engine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ScriptEngine for V8Engine {
    fn engine_type(&self) -> &str {
        if self.options.typescript {
            "v8-typescript"
        } else {
            "v8-javascript"
        }
    }

    async fn execute(
        &self,
        script: &Script,
        ctx: &ScriptContext,
    ) -> Result<ScriptValue, ScriptError> {
        // Transpile TypeScript to JavaScript if needed.
        // Use is_likely_typescript() to auto-detect TypeScript content even when
        // the engine type is set to JavaScript, providing a helpful hint.
        if script.engine != EngineType::TypeScript && is_likely_typescript(&script.source) {
            tracing::debug!(
                name = %script.name,
                "Script appears to be TypeScript but engine type is {:?}; \
                 consider using Script::typescript() for explicit TypeScript support",
                script.engine
            );
        }
        let source = if script.engine == EngineType::TypeScript && self.options.typescript {
            transpile_typescript(&script.source, Some(&script.name))?
        } else {
            script.source.clone()
        };
        let agent_id = ctx.agent_id.clone();
        let globals_map = ctx.globals.clone();
        let timeout_dur = ctx.timeout;
        let fs_config = ctx.fs_config.clone();
        let net_config = ctx.net_config.clone();
        let tool_registry = ctx.tool_registry.clone();
        let permissions = ctx.permissions.clone();
        let memory_store = ctx.memory_store.clone();
        let event_bus = ctx.event_bus.clone();
        let orchestrator = ctx.orchestrator.clone();
        let options = self.options.clone();

        // Shared slot for the V8 IsolateHandle.
        //
        // The spawn_blocking closure fills this slot immediately after creating the
        // isolate (before any user code runs).  The watchdog task reads the handle
        // and, if the deadline fires, calls `terminate_execution()` on it.  This is
        // the only correct way to interrupt a CPU-bound V8 script: tokio's
        // cooperative timeout cannot preempt a blocking thread.
        let handle_slot: Arc<Mutex<Option<v8::IsolateHandle>>> = Arc::new(Mutex::new(None));
        let handle_slot_clone = Arc::clone(&handle_slot);

        // One-shot channel: the blocking thread signals "done" so the watchdog
        // can exit cleanly without calling terminate_execution() on a dead isolate.
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

        // Watchdog task: waits for either the deadline or the "done" signal.
        let watchdog = tokio::spawn(async move {
            tokio::select! {
                _ = tokio::time::sleep(timeout_dur) => {
                    // Deadline exceeded — terminate the V8 isolate if it is still running.
                    if let Some(handle) = handle_slot_clone.lock().unwrap().take() {
                        tracing::warn!(
                            timeout_ms = timeout_dur.as_millis(),
                            "V8 script execution timed out; calling terminate_execution()"
                        );
                        handle.terminate_execution();
                    }
                }
                _ = done_rx => {
                    // Script finished before the deadline; nothing to do.
                }
            }
        });

        let result = tokio::task::spawn_blocking(move || {
            // Create V8 platform if not already initialized
            static V8_INIT: std::sync::Once = std::sync::Once::new();
            V8_INIT.call_once(|| {
                deno_core::v8::V8::initialize_platform(
                    deno_core::v8::new_default_platform(0, false).make_shared(),
                );
                deno_core::v8::V8::initialize();
            });

            // Create isolate with heap limit
            let params = v8::Isolate::create_params()
                .heap_limits(0, options.heap_limit_mb * 1024 * 1024);
            let mut isolate = v8::Isolate::new(params);

            // Publish the thread-safe handle BEFORE any user code runs so the
            // watchdog can call terminate_execution() if the deadline fires.
            {
                let isolate_handle = isolate.thread_safe_handle();
                *handle_slot.lock().unwrap() = Some(isolate_handle);
            }

            // Create handle scope
            let handle_scope = &mut v8::HandleScope::new(&mut isolate);
            let context = v8::Context::new(handle_scope);
            let scope = &mut v8::ContextScope::new(handle_scope, context);

            // Create global object
            let global = context.global(scope);

            // Inject agent_id global
            let agent_id_key = v8::String::new(scope, "agent_id").unwrap();
            let agent_id_val = v8::String::new(scope, &agent_id).unwrap();
            global.set(scope, agent_id_key.into(), agent_id_val.into());

            // Inject caller-supplied globals
            let engine = V8Engine::with_options(options.clone());
            for (key, val) in &globals_map {
                let v8_key = v8::String::new(scope, key).unwrap();
                let v8_val = engine.json_to_v8(scope, val, 0)?;
                global.set(scope, v8_key.into(), v8_val);
            }

            // Create BridgeState for bridges
            let bridge_state = BridgeState::new(
                fs_config,
                net_config,
                tool_registry,
                permissions,
                agent_id.clone(),
                memory_store,
                event_bus,
                orchestrator,
            )?;

            // Register bridges — keep the Box alive for the entire V8 execution.
            let _state_guard = super::bridge::register_bridges(scope, global, bridge_state)?;

            // Compile and execute the script
            let code = v8::String::new(scope, &source).ok_or_else(|| {
                ScriptError::Compile(CompileError::Syntax("Failed to create code string".into()))
            })?;

            let script_obj = v8::Script::compile(scope, code, None).ok_or_else(|| {
                ScriptError::Compile(CompileError::Syntax("Script compilation failed".into()))
            })?;

            // run() returns None either when a JS exception is thrown OR when
            // terminate_execution() fires.  Distinguish the two cases by checking
            // is_execution_terminating() on the scope.
            match script_obj.run(scope) {
                Some(value) => {
                    // Convert result to JSON
                    Ok::<_, ScriptError>(engine.v8_to_json(scope, value, 0))
                }
                None => {
                    if scope.is_execution_terminating() {
                        // Watchdog fired — report as Timeout.
                        Err(ScriptError::Timeout)
                    } else {
                        // Ordinary JS exception (throw, syntax error at runtime, etc.)
                        Err(ScriptError::Runtime(
                            "Script execution failed or returned exception".into(),
                        ))
                    }
                }
            }
        })
        .await
        .map_err(|e| ScriptError::Runtime(format!("Task join error: {}", e)))?;

        // Notify the watchdog that execution has finished so it can exit without
        // attempting to terminate a no-longer-running isolate.  The send may fail
        // if the watchdog already exited after firing terminate_execution(); that
        // is fine — we just discard the error.
        let _ = done_tx.send(());
        // Abort the watchdog task in case it is still sleeping on the timer.
        watchdog.abort();

        result
    }

    fn validate(&self, script: &Script) -> Result<(), ScriptError> {
        // Initialize V8 if needed
        static V8_INIT: std::sync::Once = std::sync::Once::new();
        V8_INIT.call_once(|| {
            deno_core::v8::V8::initialize_platform(
                deno_core::v8::new_default_platform(0, false).make_shared(),
            );
            deno_core::v8::V8::initialize();
        });

        // Create a temporary isolate to validate the script
        let params = v8::Isolate::create_params().heap_limits(0, 16 * 1024 * 1024); // 16MB for validation
        let mut isolate = v8::Isolate::new(params);
        let handle_scope = &mut v8::HandleScope::new(&mut isolate);
        let context = v8::Context::new(handle_scope);
        let scope = &mut v8::ContextScope::new(handle_scope, context);

        let code = v8::String::new(scope, &script.source)
            .ok_or_else(|| ScriptError::Compile(CompileError::Syntax("Invalid source".into())))?;

        // Try to compile without executing
        v8::Script::compile(scope, code, None)
            .ok_or_else(|| ScriptError::Compile(CompileError::Syntax("Syntax error".into())))?;

        Ok(())
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Script, ScriptContext};
    use serde_json::json;
    use std::time::Duration;

    fn engine() -> V8Engine {
        V8Engine::new()
    }

    fn default_ctx() -> ScriptContext {
        ScriptContext::new("test-agent")
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_execute_number() {
        let result = engine()
            .execute(&Script::javascript("t", "42"), &default_ctx())
            .await
            .unwrap();
        assert_eq!(result, json!(42));
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_execute_string() {
        let result = engine()
            .execute(&Script::javascript("t", "'hello'"), &default_ctx())
            .await
            .unwrap();
        assert_eq!(result, json!("hello"));
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_execute_boolean() {
        let result = engine()
            .execute(&Script::javascript("t", "true"), &default_ctx())
            .await
            .unwrap();
        assert_eq!(result, json!(true));
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_execute_null() {
        let result = engine()
            .execute(&Script::javascript("t", "null"), &default_ctx())
            .await
            .unwrap();
        assert!(result.is_null());
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_execute_undefined() {
        let result = engine()
            .execute(&Script::javascript("t", "let x = 1"), &default_ctx())
            .await
            .unwrap();
        // Undefined becomes null in JSON
        assert!(result.is_null());
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_return_object() {
        let result = engine()
            .execute(
                &Script::javascript("t", "({ x: 1, y: 2 })"),
                &default_ctx(),
            )
            .await
            .unwrap();
        assert!(result.is_object());
        assert_eq!(result["x"], json!(1));
        assert_eq!(result["y"], json!(2));
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_return_array() {
        let result = engine()
            .execute(&Script::javascript("t", "[1, 2, 3]"), &default_ctx())
            .await
            .unwrap();
        assert!(result.is_array());
        assert_eq!(result[0], json!(1));
        assert_eq!(result[1], json!(2));
        assert_eq!(result[2], json!(3));
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_global_injection() {
        let ctx = ScriptContext::new("agent-1").with_global("score", json!(99));
        let result = engine()
            .execute(&Script::javascript("t", "score"), &ctx)
            .await
            .unwrap();
        assert_eq!(result, json!(99));
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_agent_id_injection() {
        let ctx = ScriptContext::new("my-agent");
        let result = engine()
            .execute(&Script::javascript("t", "agent_id"), &ctx)
            .await
            .unwrap();
        assert_eq!(result, json!("my-agent"));
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_error_propagation() {
        let result = engine()
            .execute(&Script::javascript("t", "throw new Error('boom')"), &default_ctx())
            .await;
        assert!(matches!(result, Err(ScriptError::Runtime(_))));
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_timeout() {
        let ctx = ScriptContext::new("test-agent").with_timeout(Duration::from_millis(200));
        // Infinite loop — terminate_execution() must interrupt this CPU-bound loop.
        let result = engine()
            .execute(&Script::javascript("t", "while(true) {}"), &ctx)
            .await;
        // terminate_execution() fires and the engine returns ScriptError::Timeout.
        assert!(
            matches!(result, Err(ScriptError::Timeout)),
            "expected ScriptError::Timeout, got {:?}",
            result
        );
    }

    #[tokio::test]
    #[cfg(feature = "engine-v8")]
    async fn test_v8_engine_nested_object() {
        let ctx = ScriptContext::new("test-agent").with_global("data", json!({"nested": {"value": 42}}));
        let result = engine()
            .execute(&Script::javascript("t", "data.nested.value"), &ctx)
            .await
            .unwrap();
        assert_eq!(result, json!(42));
    }

    #[test]
    #[cfg(feature = "engine-v8")]
    fn test_v8_engine_validate_valid() {
        let result = engine().validate(&Script::javascript("t", "1 + 1"));
        assert!(result.is_ok());
    }

    #[test]
    #[cfg(feature = "engine-v8")]
    fn test_v8_engine_validate_invalid() {
        let result = engine().validate(&Script::javascript("t", "@#$ invalid!!!"));
        assert!(matches!(result, Err(ScriptError::Compile(_))));
    }
}
