//! V8 bridge implementation for exposing host capabilities to JavaScript/TypeScript.
//!
//! Creates a `claw` global object with sub-objects:
//! - `claw.tools`  — tool registry access (call, list, exists)
//! - `claw.log`    — logging helpers (info, warn, error)
//! - `claw.fs`     — sandboxed filesystem access (read, write, exists, list_dir, mkdir)
//! - `claw.events` — event bus (emit, poll)
//! - `claw.agent`  — agent management (spawn, status, kill, list, info)
//! - `claw.dirs`   — platform directories (config_dir, data_dir, etc.)
//! - `claw.net`    — HTTP requests (get, post)
//! - `claw.llm`    — LLM completions (complete, stream)
//!
//! Note: `claw.memory` is intentionally NOT exposed. Per the D1 architectural decision
//! (v1.3.0), memory operations belong to the application layer. Use the `claw-memory`
//! crate's Rust API directly for long-term memory and semantic search.
//!
//! # Safety
//!
//! All bridge state is stored in a `Box<BridgeState>` whose address is kept as a
//! `v8::External` on each bridge sub-object. `register_bridges` returns the Box to
//! the caller (engine.rs), which must keep it alive for the entire V8 execution.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use deno_core::v8;
use reqwest::Client;
use serde_json::{json, Value as JsonValue};

use crate::error::ScriptError;
use crate::types::{FsBridgeConfig, NetBridgeConfig};
use claw_provider::traits::LLMProvider;
use claw_runtime::{
    agent_types::{AgentConfig, AgentId, AgentStatus},
    event_bus::EventBus,
    events::Event,
    AgentOrchestrator, EventReceiver,
};
use claw_tools::{registry::ToolRegistry, types::PermissionSet};

// =============================================================================
// BridgeState
// =============================================================================

/// State container for all bridges.  Kept alive (as a `Box`) by the caller for
/// the full duration of V8 script execution.  Its address is stored as raw
/// pointers in `v8::External` values on every bridge sub-object.
pub struct BridgeState {
    pub fs_config: FsBridgeConfig,
    pub net_config: NetBridgeConfig,
    pub tool_registry: Option<Arc<ToolRegistry>>,
    pub permissions: PermissionSet,
    pub agent_id: String,
    pub event_bus: Option<Arc<EventBus>>,
    pub orchestrator: Option<Arc<AgentOrchestrator>>,
    /// Event subscription — only created when an event_bus is provided.
    pub event_rx: Option<Mutex<EventReceiver>>,
    /// Child agents spawned from this script; auto-cleaned on drop.
    pub agent_children: Mutex<Vec<AgentId>>,
    /// Shared HTTP client (connection-pool reuse across net calls).
    pub net_client: Client,
    /// LLM provider for the llm bridge.
    pub llm_provider: Option<Arc<dyn LLMProvider>>,
}

impl BridgeState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        fs_config: FsBridgeConfig,
        net_config: NetBridgeConfig,
        tool_registry: Option<Arc<ToolRegistry>>,
        permissions: PermissionSet,
        agent_id: String,
        event_bus: Option<Arc<EventBus>>,
        orchestrator: Option<Arc<AgentOrchestrator>>,
    ) -> Result<Self, crate::error::ScriptError> {
        Self::new_with_llm(
            fs_config,
            net_config,
            tool_registry,
            permissions,
            agent_id,
            event_bus,
            orchestrator,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_llm(
        fs_config: FsBridgeConfig,
        net_config: NetBridgeConfig,
        tool_registry: Option<Arc<ToolRegistry>>,
        permissions: PermissionSet,
        agent_id: String,
        event_bus: Option<Arc<EventBus>>,
        orchestrator: Option<Arc<AgentOrchestrator>>,
        llm_provider: Option<Arc<dyn LLMProvider>>,
    ) -> Result<Self, crate::error::ScriptError> {
        let event_rx = event_bus.as_ref().map(|bus| Mutex::new(bus.subscribe()));
        let timeout = Duration::from_secs(if net_config.timeout_secs > 0 {
            net_config.timeout_secs
        } else {
            30
        });
        let net_client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| crate::error::ScriptError::Setup(format!("failed to build HTTP client: {}", e)))?;
        Ok(Self {
            fs_config,
            net_config,
            tool_registry,
            permissions,
            agent_id,
            event_bus,
            orchestrator,
            event_rx,
            agent_children: Mutex::new(Vec::new()),
            net_client,
            llm_provider,
        })
    }
}

impl Drop for BridgeState {
    fn drop(&mut self) {
        // Auto-cleanup: unregister every child agent spawned by this script.
        if let Some(orc) = &self.orchestrator {
            if let Ok(children) = self.agent_children.lock() {
                for id in children.iter() {
                    let _ = orc.unregister(id, "script ended");
                }
            }
        }
    }
}

// =============================================================================
// Entry point
// =============================================================================

/// Register all bridges on the V8 global `claw` object.
///
/// Returns the `Box<BridgeState>` that the caller **must** keep alive for the
/// entire V8 execution (store in `let _guard = register_bridges(...)?`).
pub fn register_bridges(
    scope: &mut v8::HandleScope<'_>,
    global: v8::Local<v8::Object>,
    state: BridgeState,
) -> Result<Box<BridgeState>, ScriptError> {
    let boxed = Box::new(state);
    // SAFETY: we return the Box to the caller; it stays alive until script ends.
    let ptr = &*boxed as *const BridgeState as *mut std::ffi::c_void;

    let claw_key = v8::String::new(scope, "claw").unwrap();
    let claw_obj = v8::Object::new(scope);

    register_tools_bridge(scope, claw_obj, ptr, &boxed)?;
    register_log_bridge(scope, claw_obj)?;
    register_fs_bridge(scope, claw_obj, ptr)?;
    register_events_bridge(scope, claw_obj, ptr)?;
    register_agent_bridge(scope, claw_obj, ptr)?;
    register_dirs_bridge(scope, claw_obj)?;
    register_net_bridge(scope, claw_obj, ptr)?;
    register_llm_bridge(scope, claw_obj, ptr)?;

    global.set(scope, claw_key.into(), claw_obj.into());
    Ok(boxed)
}

// =============================================================================
// Helper: v8::Value → serde_json::Value
// =============================================================================

pub fn js_value_to_json(
    scope: &mut v8::HandleScope<'_>,
    value: v8::Local<v8::Value>,
) -> serde_json::Value {
    js_value_depth(scope, value, 0)
}

fn js_value_depth(
    scope: &mut v8::HandleScope<'_>,
    value: v8::Local<v8::Value>,
    depth: u32,
) -> serde_json::Value {
    if depth > 16 {
        return serde_json::Value::Null;
    }
    if value.is_null() || value.is_undefined() {
        serde_json::Value::Null
    } else if value.is_boolean() {
        serde_json::Value::Bool(value.is_true())
    } else if value.is_number() {
        let n = value.number_value(scope).unwrap_or(0.0);
        if n.fract() == 0.0 {
            serde_json::Value::Number(serde_json::Number::from(n as i64))
        } else {
            serde_json::Number::from_f64(n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
        }
    } else if value.is_string() {
        let s = value
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default();
        serde_json::Value::String(s)
    } else if value.is_array() {
        let arr = v8::Local::<v8::Array>::try_from(value).unwrap();
        let len = arr.length();
        let mut vec = Vec::with_capacity(len as usize);
        for i in 0..len {
            let elem = arr
                .get_index(scope, i)
                .unwrap_or_else(|| v8::undefined(scope).into());
            vec.push(js_value_depth(scope, elem, depth + 1));
        }
        serde_json::Value::Array(vec)
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
                    let val = obj
                        .get(scope, key)
                        .unwrap_or_else(|| v8::undefined(scope).into());
                    map.insert(key_str, js_value_depth(scope, val, depth + 1));
                }
            }
        }
        serde_json::Value::Object(map)
    } else {
        serde_json::Value::Null
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Set `{ success: false, error: msg }` on a result object.
fn set_error(scope: &mut v8::HandleScope<'_>, obj: v8::Local<v8::Object>, msg: &str) {
    let k = v8::String::new(scope, "success").unwrap();
    let v = v8::Boolean::new(scope, false);
    obj.set(scope, k.into(), v.into());
    let k = v8::String::new(scope, "error").unwrap();
    let v = v8::String::new(scope, msg).unwrap();
    obj.set(scope, k.into(), v.into());
}

/// Attach a function template as a named property on an object.
fn set_fn(
    scope: &mut v8::HandleScope<'_>,
    obj: v8::Local<v8::Object>,
    key: &str,
    tpl: v8::Local<v8::FunctionTemplate>,
) {
    let k = v8::String::new(scope, key).unwrap();
    let f = tpl.get_function(scope).unwrap();
    obj.set(scope, k.into(), f.into());
}

/// Store `*mut c_void` as an `v8::External` under `"_s"` on `obj`.
fn store_state(
    scope: &mut v8::HandleScope<'_>,
    obj: v8::Local<v8::Object>,
    ptr: *mut std::ffi::c_void,
) {
    let ext = v8::External::new(scope, ptr);
    let key = v8::String::new(scope, "_s").unwrap();
    obj.set(scope, key.into(), ext.into());
}

/// Retrieve a `&BridgeState` from the `"_s"` External on `this`.
///
/// # Safety
/// Caller must ensure the pointed-to `BridgeState` is still alive.
unsafe fn get_state<'a>(
    scope: &mut v8::HandleScope<'_>,
    this: v8::Local<v8::Object>,
) -> Option<&'a BridgeState> {
    let key = v8::String::new(scope, "_s").unwrap();
    let val = this.get(scope, key.into())?;
    if !val.is_external() {
        return None;
    }
    let ext = v8::Local::<v8::External>::try_from(val).ok()?;
    Some(&*(ext.value() as *const BridgeState))
}

// =============================================================================
// Tools Bridge — claw.tools.{call, list, exists}
// =============================================================================

fn register_tools_bridge(
    scope: &mut v8::HandleScope<'_>,
    claw_obj: v8::Local<v8::Object>,
    ptr: *mut std::ffi::c_void,
    state: &BridgeState,
) -> Result<(), ScriptError> {
    let tools_obj = v8::Object::new(scope);
    store_state(scope, tools_obj, ptr);

    if state.tool_registry.is_some() {
        // ── claw.tools.call(name, params?) ──────────────────────────────────
        let call_tpl = v8::FunctionTemplate::new(
            scope,
            |scope: &mut v8::HandleScope<'_>,
             args: v8::FunctionCallbackArguments<'_>,
             mut rv: v8::ReturnValue<'_>| {
                let result_obj = v8::Object::new(scope);

                let state = match unsafe { get_state(scope, args.this()) } {
                    Some(s) => s,
                    None => {
                        set_error(scope, result_obj, "bridge state unavailable");
                        rv.set(result_obj.into());
                        return;
                    }
                };

                let registry = match &state.tool_registry {
                    Some(r) => r,
                    None => {
                        set_error(scope, result_obj, "no tool registry configured");
                        rv.set(result_obj.into());
                        return;
                    }
                };

                let name = args
                    .get(0)
                    .to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_default();

                let params = if args.length() > 1 {
                    js_value_to_json(scope, args.get(1))
                } else {
                    json!({})
                };

                use claw_tools::types::ToolContext;
                let ctx = ToolContext::new(&state.agent_id, PermissionSet::minimal());

                let outcome = tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current()
                        .block_on(registry.execute(&name, params, ctx))
                });

                match outcome {
                    Ok(res) if res.success => {
                        let k = v8::String::new(scope, "success").unwrap();
                        let v = v8::Boolean::new(scope, true);
                        result_obj.set(scope, k.into(), v.into());
                        if let Some(output) = res.output {
                            let out_str = serde_json::to_string(&output).unwrap_or_default();
                            let k = v8::String::new(scope, "output").unwrap();
                            let v = v8::String::new(scope, &out_str).unwrap();
                            result_obj.set(scope, k.into(), v.into());
                        }
                    }
                    Ok(res) => {
                        let msg = res
                            .error
                            .map(|e| e.message)
                            .unwrap_or_else(|| "tool failed".to_string());
                        set_error(scope, result_obj, &msg);
                    }
                    Err(e) => set_error(scope, result_obj, &e.to_string()),
                }
                rv.set(result_obj.into());
            },
        );
        set_fn(scope, tools_obj, "call", call_tpl);

        // ── claw.tools.list() ───────────────────────────────────────────────
        let list_tpl = v8::FunctionTemplate::new(
            scope,
            |scope: &mut v8::HandleScope<'_>,
             args: v8::FunctionCallbackArguments<'_>,
             mut rv: v8::ReturnValue<'_>| {
                let empty = v8::Array::new(scope, 0);
                let state = match unsafe { get_state(scope, args.this()) } {
                    Some(s) => s,
                    None => {
                        rv.set(empty.into());
                        return;
                    }
                };
                let names = match &state.tool_registry {
                    Some(r) => r.tool_names(),
                    None => vec![],
                };
                let arr = v8::Array::new(scope, names.len() as i32);
                for (i, name) in names.iter().enumerate() {
                    let v = v8::String::new(scope, name).unwrap();
                    arr.set_index(scope, i as u32, v.into());
                }
                rv.set(arr.into());
            },
        );
        set_fn(scope, tools_obj, "list", list_tpl);

        // ── claw.tools.exists(name) ─────────────────────────────────────────
        let exists_tpl = v8::FunctionTemplate::new(
            scope,
            |scope: &mut v8::HandleScope<'_>,
             args: v8::FunctionCallbackArguments<'_>,
             mut rv: v8::ReturnValue<'_>| {
                let state = match unsafe { get_state(scope, args.this()) } {
                    Some(s) => s,
                    None => {
                        rv.set(v8::Boolean::new(scope, false).into());
                        return;
                    }
                };
                let name = args
                    .get(0)
                    .to_string(scope)
                    .map(|s| s.to_rust_string_lossy(scope))
                    .unwrap_or_default();
                let found = state
                    .tool_registry
                    .as_ref()
                    .map(|r| r.tool_meta(&name).is_some())
                    .unwrap_or(false);
                rv.set(v8::Boolean::new(scope, found).into());
            },
        );
        set_fn(scope, tools_obj, "exists", exists_tpl);
    } else {
        // No registry — stubs
        let call_tpl = v8::FunctionTemplate::new(
            scope,
            |scope: &mut v8::HandleScope<'_>,
             _args: v8::FunctionCallbackArguments<'_>,
             mut rv: v8::ReturnValue<'_>| {
                let obj = v8::Object::new(scope);
                set_error(scope, obj, "no tool registry configured");
                rv.set(obj.into());
            },
        );
        set_fn(scope, tools_obj, "call", call_tpl);

        let list_tpl = v8::FunctionTemplate::new(
            scope,
            |scope: &mut v8::HandleScope<'_>,
             _args: v8::FunctionCallbackArguments<'_>,
             mut rv: v8::ReturnValue<'_>| {
                rv.set(v8::Array::new(scope, 0).into());
            },
        );
        set_fn(scope, tools_obj, "list", list_tpl);

        let exists_tpl = v8::FunctionTemplate::new(
            scope,
            |scope: &mut v8::HandleScope<'_>,
             _args: v8::FunctionCallbackArguments<'_>,
             mut rv: v8::ReturnValue<'_>| {
                rv.set(v8::Boolean::new(scope, false).into());
            },
        );
        set_fn(scope, tools_obj, "exists", exists_tpl);
    }

    let k = v8::String::new(scope, "tools").unwrap();
    claw_obj.set(scope, k.into(), tools_obj.into());
    Ok(())
}

// =============================================================================
// Log Bridge — claw.log.{info, warn, error}
// =============================================================================

fn register_log_bridge(
    scope: &mut v8::HandleScope<'_>,
    claw_obj: v8::Local<v8::Object>,
) -> Result<(), ScriptError> {
    let log_obj = v8::Object::new(scope);

    let info_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         _rv: v8::ReturnValue<'_>| {
            let msg = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            tracing::info!(target: "claw.script", "[claw.log.info] {}", msg);
        },
    );
    set_fn(scope, log_obj, "info", info_tpl);

    let warn_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         _rv: v8::ReturnValue<'_>| {
            let msg = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            tracing::warn!(target: "claw.script", "[claw.log.warn] {}", msg);
        },
    );
    set_fn(scope, log_obj, "warn", warn_tpl);

    let error_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         _rv: v8::ReturnValue<'_>| {
            let msg = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            tracing::error!(target: "claw.script", "[claw.log.error] {}", msg);
        },
    );
    set_fn(scope, log_obj, "error", error_tpl);

    let k = v8::String::new(scope, "log").unwrap();
    claw_obj.set(scope, k.into(), log_obj.into());
    Ok(())
}

// =============================================================================
// FS Bridge — claw.fs.{read, write, exists, list_dir, mkdir}
// =============================================================================

/// Validate and resolve a path against the fs_config sandbox.
///
/// Returns the canonicalized path or an error string.
fn fs_validate_path(config: &FsBridgeConfig, path: &str) -> Result<PathBuf, String> {
    if config.allowed_paths.is_empty() {
        return Err(format!(
            "Permission denied: no filesystem access allowed (path: '{}')",
            path
        ));
    }
    let path_obj = Path::new(path);
    let resolved = if path_obj.is_absolute() {
        path_obj.to_path_buf()
    } else {
        config.base_dir.join(path_obj)
    };

    let canonical = match resolved.canonicalize() {
        Ok(c) => c,
        Err(_) => {
            // For non-existent paths, normalise and check traversal.
            let base_canonical = config
                .base_dir
                .canonicalize()
                .map_err(|e| format!("Failed to resolve base directory: {}", e))?;
            let normalized = fs_normalize_path(&resolved);
            if !path_obj.is_absolute() && !normalized.starts_with(&base_canonical) {
                return Err(format!(
                    "Permission denied: path '{}' is outside allowed directories",
                    path
                ));
            }
            return Err(format!(
                "Failed to resolve path '{}': No such file or directory",
                path
            ));
        }
    };

    for allowed in &config.allowed_paths {
        let allowed_canonical = allowed.canonicalize().unwrap_or_else(|_| allowed.clone());
        if canonical.starts_with(&allowed_canonical) {
            return Ok(canonical);
        }
    }
    Err(format!(
        "Permission denied: path '{}' is outside allowed directories",
        canonical.display()
    ))
}

fn fs_normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::Normal(c) => result.push(c),
            std::path::Component::RootDir => result.push("/"),
            _ => {}
        }
    }
    result
}

fn register_fs_bridge(
    scope: &mut v8::HandleScope<'_>,
    claw_obj: v8::Local<v8::Object>,
    ptr: *mut std::ffi::c_void,
) -> Result<(), ScriptError> {
    let fs_obj = v8::Object::new(scope);
    store_state(scope, fs_obj, ptr);

    // ── claw.fs.read(path) → string ──────────────────────────────────────────
    let read_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
            };
            let path = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            match fs_validate_path(&state.fs_config, &path)
                .and_then(|p| std::fs::read_to_string(&p).map_err(|e| e.to_string()))
            {
                Ok(content) => {
                    let v = v8::String::new(scope, &content).unwrap();
                    rv.set(v.into());
                }
                Err(e) => {
                    let msg = v8::String::new(scope, &e).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                }
            }
        },
    );
    set_fn(scope, fs_obj, "read", read_tpl);

    // ── claw.fs.write(path, content) ─────────────────────────────────────────
    let write_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
            };
            let path = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let content = args
                .get(1)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            // For write, validate parent dir (file may not exist yet).
            let path_obj = Path::new(&path);
            let resolved = if path_obj.is_absolute() {
                path_obj.to_path_buf()
            } else {
                state.fs_config.base_dir.join(path_obj)
            };
            // Validate parent directory is within allowed paths.
            let parent = resolved.parent().unwrap_or(&resolved).to_path_buf();
            let parent_canonical = parent.canonicalize().unwrap_or_else(|_| parent.clone());
            let allowed = state.fs_config.allowed_paths.iter().any(|a| {
                let ac = a.canonicalize().unwrap_or_else(|_| a.clone());
                parent_canonical.starts_with(&ac)
            });
            if !allowed || state.fs_config.allowed_paths.is_empty() {
                let msg = v8::String::new(scope, "Permission denied: path outside allowed dirs")
                    .unwrap();
                let exc = v8::Exception::error(scope, msg);
                scope.throw_exception(exc);
                return;
            }
            match std::fs::write(&resolved, content.as_bytes()) {
                Ok(_) => rv.set(v8::undefined(scope).into()),
                Err(e) => {
                    let msg = v8::String::new(scope, &e.to_string()).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                }
            }
        },
    );
    set_fn(scope, fs_obj, "write", write_tpl);

    // ── claw.fs.exists(path) → bool ──────────────────────────────────────────
    let exists_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::Boolean::new(scope, false).into());
                    return;
                }
            };
            let path = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            // For exists, simply resolve and check.
            let resolved = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                state.fs_config.base_dir.join(&path)
            };
            let exists = resolved.exists();
            rv.set(v8::Boolean::new(scope, exists).into());
        },
    );
    set_fn(scope, fs_obj, "exists", exists_tpl);

    // ── claw.fs.list_dir(path) → string[] ───────────────────────────────────
    let list_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::Array::new(scope, 0).into());
                    return;
                }
            };
            let path = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            match fs_validate_path(&state.fs_config, &path).and_then(|p| {
                std::fs::read_dir(&p)
                    .map_err(|e| e.to_string())
                    .map(|entries| {
                        entries
                            .filter_map(|e| {
                                e.ok()
                                    .and_then(|e| e.file_name().into_string().ok())
                            })
                            .collect::<Vec<_>>()
                    })
            }) {
                Ok(names) => {
                    let arr = v8::Array::new(scope, names.len() as i32);
                    for (i, name) in names.iter().enumerate() {
                        let v = v8::String::new(scope, name).unwrap();
                        arr.set_index(scope, i as u32, v.into());
                    }
                    rv.set(arr.into());
                }
                Err(e) => {
                    let msg = v8::String::new(scope, &e).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                }
            }
        },
    );
    set_fn(scope, fs_obj, "list_dir", list_tpl);

    // ── claw.fs.mkdir(path) ───────────────────────────────────────────────────
    let mkdir_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
            };
            let path = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            if state.fs_config.allowed_paths.is_empty() {
                let msg = v8::String::new(scope, "Permission denied: no filesystem access").unwrap();
                let exc = v8::Exception::error(scope, msg);
                scope.throw_exception(exc);
                return;
            }
            let resolved = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                state.fs_config.base_dir.join(&path)
            };
            match std::fs::create_dir_all(&resolved) {
                Ok(_) => rv.set(v8::undefined(scope).into()),
                Err(e) => {
                    let msg = v8::String::new(scope, &e.to_string()).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                }
            }
        },
    );
    set_fn(scope, fs_obj, "mkdir", mkdir_tpl);

    let k = v8::String::new(scope, "fs").unwrap();
    claw_obj.set(scope, k.into(), fs_obj.into());
    Ok(())
}

// =============================================================================
// Events Bridge — claw.events.{emit, poll}
// =============================================================================

/// Convert an `Event` to `(type_string, JsonValue)` data pair.
fn event_to_parts(event: &Event) -> (String, JsonValue) {
    match event {
        Event::AgentStarted { agent_id } => (
            "agent_started".into(),
            json!({ "agent_id": agent_id.as_str() }),
        ),
        Event::AgentStopped { agent_id, reason } => (
            "agent_stopped".into(),
            json!({ "agent_id": agent_id.as_str(), "reason": reason }),
        ),
        Event::LlmRequestStarted { agent_id, provider } => (
            "llm_request_started".into(),
            json!({ "agent_id": agent_id.as_str(), "provider": provider }),
        ),
        Event::LlmRequestCompleted {
            agent_id,
            prompt_tokens,
            completion_tokens,
        } => (
            "llm_request_completed".into(),
            json!({
                "agent_id": agent_id.as_str(),
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
            }),
        ),
        Event::ToolCalled {
            agent_id,
            tool_name,
            call_id,
        } => (
            "tool_called".into(),
            json!({
                "agent_id": agent_id.as_str(),
                "tool_name": tool_name,
                "call_id": call_id,
            }),
        ),
        Event::ToolResult {
            agent_id,
            tool_name,
            call_id,
            success,
        } => (
            "tool_result".into(),
            json!({
                "agent_id": agent_id.as_str(),
                "tool_name": tool_name,
                "call_id": call_id,
                "success": success,
            }),
        ),
        Event::Shutdown => ("shutdown".into(), json!({})),
        Event::Custom { event_type, data } => (event_type.clone(), data.clone()),
        _ => ("unknown".into(), json!({})),
    }
}

fn register_events_bridge(
    scope: &mut v8::HandleScope<'_>,
    claw_obj: v8::Local<v8::Object>,
    ptr: *mut std::ffi::c_void,
) -> Result<(), ScriptError> {
    let ev_obj = v8::Object::new(scope);
    store_state(scope, ev_obj, ptr);

    // ── claw.events.emit(type, data?) ────────────────────────────────────────
    let emit_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
            };
            let event_type = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let data = if args.length() > 1 {
                js_value_to_json(scope, args.get(1))
            } else {
                json!({})
            };
            if let Some(bus) = &state.event_bus {
                let _ = bus.publish(Event::Custom { event_type, data });
            }
            rv.set(v8::undefined(scope).into());
        },
    );
    set_fn(scope, ev_obj, "emit", emit_tpl);

    // ── claw.events.poll() → {type, data}[] ─────────────────────────────────
    let poll_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::Array::new(scope, 0).into());
                    return;
                }
            };

            let pending: Vec<(String, JsonValue)> = match &state.event_rx {
                Some(rx_lock) => {
                    let mut rx = rx_lock.lock().unwrap();
                    let mut events = Vec::new();
                    while let Ok(event) = rx.try_recv() {
                        events.push(event_to_parts(&event));
                    }
                    events
                }
                None => vec![],
            };

            let arr = v8::Array::new(scope, pending.len() as i32);
            for (i, (type_str, data)) in pending.iter().enumerate() {
                let item_obj = v8::Object::new(scope);
                // type
                let t_k = v8::String::new(scope, "type").unwrap();
                let t_v = v8::String::new(scope, type_str).unwrap();
                item_obj.set(scope, t_k.into(), t_v.into());
                // data (JSON string)
                let d_k = v8::String::new(scope, "data").unwrap();
                let d_str = serde_json::to_string(data).unwrap_or_default();
                let d_v = v8::String::new(scope, &d_str).unwrap();
                item_obj.set(scope, d_k.into(), d_v.into());
                arr.set_index(scope, i as u32, item_obj.into());
            }
            rv.set(arr.into());
        },
    );
    set_fn(scope, ev_obj, "poll", poll_tpl);

    let k = v8::String::new(scope, "events").unwrap();
    claw_obj.set(scope, k.into(), ev_obj.into());
    Ok(())
}

// =============================================================================
// Agent Bridge — claw.agent.{spawn, status, kill, list, info}
// =============================================================================

fn agent_status_str(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Starting => "starting",
        AgentStatus::Running => "running",
        AgentStatus::Paused => "paused",
        AgentStatus::Stopped => "stopped",
        AgentStatus::Error => "error",
    }
}

fn register_agent_bridge(
    scope: &mut v8::HandleScope<'_>,
    claw_obj: v8::Local<v8::Object>,
    ptr: *mut std::ffi::c_void,
) -> Result<(), ScriptError> {
    let ag_obj = v8::Object::new(scope);
    store_state(scope, ag_obj, ptr);

    // ── claw.agent.spawn(name) → string ──────────────────────────────────────
    let spawn_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::null(scope).into());
                    return;
                }
            };
            let orc = match &state.orchestrator {
                Some(o) => Arc::clone(o),
                None => {
                    rv.set(v8::null(scope).into());
                    return;
                }
            };
            let name = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "unnamed".to_string());
            let cfg = AgentConfig::new(name);
            let agent_id = cfg.agent_id.clone();
            match orc.register(cfg) {
                Ok(_) => {
                    // Track child for auto-cleanup.
                    if let Ok(mut children) = state.agent_children.lock() {
                        children.push(agent_id.clone());
                    }
                    let v = v8::String::new(scope, agent_id.as_str()).unwrap();
                    rv.set(v.into());
                }
                Err(e) => {
                    let msg = v8::String::new(scope, &e.to_string()).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                }
            }
        },
    );
    set_fn(scope, ag_obj, "spawn", spawn_tpl);

    // ── claw.agent.status(id) → string ───────────────────────────────────────
    let status_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    let v = v8::String::new(scope, "unknown").unwrap();
                    rv.set(v.into());
                    return;
                }
            };
            let id_str = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let id = AgentId::new(id_str);
            let status = state
                .orchestrator
                .as_ref()
                .and_then(|o| o.agent_info(&id))
                .map(|info| agent_status_str(info.status))
                .unwrap_or("unknown");
            let v = v8::String::new(scope, status).unwrap();
            rv.set(v.into());
        },
    );
    set_fn(scope, ag_obj, "status", status_tpl);

    // ── claw.agent.kill(id) ───────────────────────────────────────────────────
    let kill_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::undefined(scope).into());
                    return;
                }
            };
            let id_str = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let id = AgentId::new(&id_str);
            if let Some(orc) = &state.orchestrator {
                let _ = orc.unregister(&id, "killed by script");
            }
            // Remove from children list.
            if let Ok(mut children) = state.agent_children.lock() {
                children.retain(|c| c.as_str() != id_str);
            }
            rv.set(v8::undefined(scope).into());
        },
    );
    set_fn(scope, ag_obj, "kill", kill_tpl);

    // ── claw.agent.list() → string[] ─────────────────────────────────────────
    let list_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::Array::new(scope, 0).into());
                    return;
                }
            };
            let children: Vec<String> = state
                .agent_children
                .lock()
                .map(|c| c.iter().map(|id| id.0.clone()).collect())
                .unwrap_or_default();
            let arr = v8::Array::new(scope, children.len() as i32);
            for (i, id) in children.iter().enumerate() {
                let v = v8::String::new(scope, id).unwrap();
                arr.set_index(scope, i as u32, v.into());
            }
            rv.set(arr.into());
        },
    );
    set_fn(scope, ag_obj, "list", list_tpl);

    // ── claw.agent.info(id) → {id,name,status,started_at} | null ────────────
    let info_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    rv.set(v8::null(scope).into());
                    return;
                }
            };
            let id_str = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let id = AgentId::new(&id_str);
            match state.orchestrator.as_ref().and_then(|o| o.agent_info(&id)) {
                Some(info) => {
                    let obj = v8::Object::new(scope);
                    let id_k = v8::String::new(scope, "id").unwrap();
                    let id_v = v8::String::new(scope, info.config.agent_id.as_str()).unwrap();
                    obj.set(scope, id_k.into(), id_v.into());
                    let name_k = v8::String::new(scope, "name").unwrap();
                    let name_v = v8::String::new(scope, &info.config.name).unwrap();
                    obj.set(scope, name_k.into(), name_v.into());
                    let status_k = v8::String::new(scope, "status").unwrap();
                    let status_v =
                        v8::String::new(scope, agent_status_str(info.status)).unwrap();
                    obj.set(scope, status_k.into(), status_v.into());
                    let ts_k = v8::String::new(scope, "started_at").unwrap();
                    let ts_v = v8::Number::new(scope, info.started_at as f64);
                    obj.set(scope, ts_k.into(), ts_v.into());
                    rv.set(obj.into());
                }
                None => rv.set(v8::null(scope).into()),
            }
        },
    );
    set_fn(scope, ag_obj, "info", info_tpl);

    let k = v8::String::new(scope, "agent").unwrap();
    claw_obj.set(scope, k.into(), ag_obj.into());
    Ok(())
}

// =============================================================================
// Dirs Bridge — claw.dirs.{config_dir, data_dir, cache_dir, tools_dir,
//                           scripts_dir, logs_dir}
// =============================================================================

fn register_dirs_bridge(
    scope: &mut v8::HandleScope<'_>,
    claw_obj: v8::Local<v8::Object>,
) -> Result<(), ScriptError> {
    use claw_runtime::dirs;

    let dirs_obj = v8::Object::new(scope);

    macro_rules! dir_fn {
        ($scope:expr, $obj:expr, $key:expr, $fn:expr) => {{
            let tpl = v8::FunctionTemplate::new(
                $scope,
                |scope: &mut v8::HandleScope<'_>,
                 _args: v8::FunctionCallbackArguments<'_>,
                 mut rv: v8::ReturnValue<'_>| {
                    match $fn() {
                        Some(p) => {
                            let s = p.to_string_lossy();
                            let v = v8::String::new(scope, s.as_ref()).unwrap();
                            rv.set(v.into());
                        }
                        None => rv.set(v8::null(scope).into()),
                    }
                },
            );
            set_fn($scope, $obj, $key, tpl);
        }};
    }

    dir_fn!(scope, dirs_obj, "config_dir", dirs::config_dir);
    dir_fn!(scope, dirs_obj, "data_dir", dirs::data_dir);
    dir_fn!(scope, dirs_obj, "cache_dir", dirs::cache_dir);
    dir_fn!(scope, dirs_obj, "tools_dir", dirs::tools_dir);
    dir_fn!(scope, dirs_obj, "scripts_dir", dirs::scripts_dir);
    dir_fn!(scope, dirs_obj, "logs_dir", dirs::logs_dir);

    let k = v8::String::new(scope, "dirs").unwrap();
    claw_obj.set(scope, k.into(), dirs_obj.into());
    Ok(())
}

// =============================================================================
// Net Bridge — claw.net.{get, post}
// =============================================================================

/// Validate a URL against the net_config sandbox.
fn net_validate_url(config: &NetBridgeConfig, url: &str) -> Result<reqwest::Url, String> {
    let parsed = url
        .parse::<reqwest::Url>()
        .map_err(|e| format!("Invalid URL '{}': {}", url, e))?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(format!(
            "Unsupported URL scheme '{}': only http and https are allowed",
            scheme
        ));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| format!("URL '{}' has no host", url))?;

    let is_loopback = host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false);

    if is_loopback && !config.allow_loopback {
        return Err(format!(
            "Permission denied: loopback access not allowed (host: '{}')",
            host
        ));
    }

    if !is_loopback || !config.allow_loopback {
        if config.allowed_domains.is_empty() {
            return Err(format!(
                "Permission denied: no network access allowed (URL: '{}')",
                url
            ));
        }
        let allowed = config.allowed_domains.iter().any(|d| {
            host == d.as_str() || host.ends_with(&format!(".{}", d))
        });
        if !allowed {
            return Err(format!(
                "Permission denied: domain '{}' is not in the allowlist",
                host
            ));
        }
    }

    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| format!("URL '{}' has no port", url))?;

    let allowed_ports = if config.allowed_ports.is_empty() {
        std::collections::HashSet::from([80u16, 443u16])
    } else {
        config.allowed_ports.clone()
    };

    if !allowed_ports.contains(&port) {
        return Err(format!(
            "Permission denied: port {} is not allowed (allowed: {:?})",
            port, allowed_ports
        ));
    }

    Ok(parsed)
}

fn register_net_bridge(
    scope: &mut v8::HandleScope<'_>,
    claw_obj: v8::Local<v8::Object>,
    ptr: *mut std::ffi::c_void,
) -> Result<(), ScriptError> {
    let net_obj = v8::Object::new(scope);
    store_state(scope, net_obj, ptr);

    // ── claw.net.get(url, headers?) → {status, body} ─────────────────────────
    let get_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    let obj = v8::Object::new(scope);
                    set_error(scope, obj, "bridge state unavailable");
                    rv.set(obj.into());
                    return;
                }
            };
            let url_str = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();

            let validated = match net_validate_url(&state.net_config, &url_str) {
                Ok(u) => u,
                Err(e) => {
                    let obj = v8::Object::new(scope);
                    set_error(scope, obj, &e);
                    rv.set(obj.into());
                    return;
                }
            };

            // Optional headers from second arg (plain JS object).
            let mut req_headers = reqwest::header::HeaderMap::new();
            if args.length() > 1 && args.get(1).is_object() {
                let headers_json = js_value_to_json(scope, args.get(1));
                if let JsonValue::Object(map) = headers_json {
                    for (k, v) in map {
                        if let JsonValue::String(v_str) = v {
                            if let (Ok(hn), Ok(hv)) = (
                                k.parse::<reqwest::header::HeaderName>(),
                                v_str.parse::<reqwest::header::HeaderValue>(),
                            ) {
                                req_headers.insert(hn, hv);
                            }
                        }
                    }
                }
            }

            let client = state.net_client.clone();
            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    client
                        .get(validated)
                        .headers(req_headers)
                        .send()
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|resp| {
                            let status = resp.status().as_u16();
                            Ok((status, resp))
                        })
                })
            });

            match result {
                Ok((status, resp)) => {
                    let body = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(resp.text())
                            .unwrap_or_default()
                    });
                    let obj = v8::Object::new(scope);
                    let s_k = v8::String::new(scope, "status").unwrap();
                    let s_v = v8::Number::new(scope, status as f64);
                    obj.set(scope, s_k.into(), s_v.into());
                    let b_k = v8::String::new(scope, "body").unwrap();
                    let b_v = v8::String::new(scope, &body).unwrap();
                    obj.set(scope, b_k.into(), b_v.into());
                    rv.set(obj.into());
                }
                Err(e) => {
                    let obj = v8::Object::new(scope);
                    set_error(scope, obj, &e);
                    rv.set(obj.into());
                }
            }
        },
    );
    set_fn(scope, net_obj, "get", get_tpl);

    // ── claw.net.post(url, body, headers?) → {status, body} ──────────────────
    let post_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    let obj = v8::Object::new(scope);
                    set_error(scope, obj, "bridge state unavailable");
                    rv.set(obj.into());
                    return;
                }
            };
            let url_str = args
                .get(0)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();
            let body_str = args
                .get(1)
                .to_string(scope)
                .map(|s| s.to_rust_string_lossy(scope))
                .unwrap_or_default();

            let validated = match net_validate_url(&state.net_config, &url_str) {
                Ok(u) => u,
                Err(e) => {
                    let obj = v8::Object::new(scope);
                    set_error(scope, obj, &e);
                    rv.set(obj.into());
                    return;
                }
            };

            let mut req_headers = reqwest::header::HeaderMap::new();
            if args.length() > 2 && args.get(2).is_object() {
                let headers_json = js_value_to_json(scope, args.get(2));
                if let JsonValue::Object(map) = headers_json {
                    for (k, v) in map {
                        if let JsonValue::String(v_str) = v {
                            if let (Ok(hn), Ok(hv)) = (
                                k.parse::<reqwest::header::HeaderName>(),
                                v_str.parse::<reqwest::header::HeaderValue>(),
                            ) {
                                req_headers.insert(hn, hv);
                            }
                        }
                    }
                }
            }

            let client = state.net_client.clone();
            let result = tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    client
                        .post(validated)
                        .headers(req_headers)
                        .body(body_str)
                        .send()
                        .await
                        .map_err(|e| e.to_string())
                })
            });

            match result {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    let body = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(resp.text())
                            .unwrap_or_default()
                    });
                    let obj = v8::Object::new(scope);
                    let s_k = v8::String::new(scope, "status").unwrap();
                    let s_v = v8::Number::new(scope, status as f64);
                    obj.set(scope, s_k.into(), s_v.into());
                    let b_k = v8::String::new(scope, "body").unwrap();
                    let b_v = v8::String::new(scope, &body).unwrap();
                    obj.set(scope, b_k.into(), b_v.into());
                    rv.set(obj.into());
                }
                Err(e) => {
                    let obj = v8::Object::new(scope);
                    set_error(scope, obj, &e);
                    rv.set(obj.into());
                }
            }
        },
    );
    set_fn(scope, net_obj, "post", post_tpl);

    let k = v8::String::new(scope, "net").unwrap();
    claw_obj.set(scope, k.into(), net_obj.into());
    Ok(())
}

// =============================================================================
// LLM Bridge — claw.llm.{complete, stream}
// =============================================================================
//
// Both methods are synchronous from JS's perspective (they run inside
// spawn_blocking) and call block_on to drive the async LLMProvider future.
//
// complete(messages, opts?) -> string
//   messages: Array<{role: string, content: string}>
//   opts?: {model?: string, max_tokens?: number, temperature?: number}
//   returns: assistant content string, or throws on error
//
// stream(messages, opts?) -> Array<string>
//   Same arguments; collects all streaming deltas and returns them as an array.

fn register_llm_bridge(
    scope: &mut v8::HandleScope<'_>,
    claw_obj: v8::Local<v8::Object>,
    ptr: *mut std::ffi::c_void,
) -> Result<(), ScriptError> {
    let llm_obj = v8::Object::new(scope);
    store_state(scope, llm_obj, ptr);

    // ── claw.llm.complete(messages, opts?) ────────────────────────────────────
    let complete_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    let msg = v8::String::new(scope, "bridge state unavailable").unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                    return;
                }
            };

            let provider = match &state.llm_provider {
                Some(p) => Arc::clone(p),
                None => {
                    let msg = v8::String::new(scope, "llm bridge: no provider configured").unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                    return;
                }
            };

            // Parse messages (arg 0)
            let messages_val = args.get(0);
            let messages = match llm_parse_messages(scope, messages_val) {
                Ok(m) => m,
                Err(e) => {
                    let msg = v8::String::new(scope, &e).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                    return;
                }
            };

            // Parse opts (arg 1, optional)
            let opts_val = args.get(1);
            let options = match llm_parse_opts(scope, opts_val, provider.model_id()) {
                Ok(o) => o,
                Err(e) => {
                    let msg = v8::String::new(scope, &e).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                    return;
                }
            };

            // Drive async via block_on (we are inside spawn_blocking)
            let handle = tokio::runtime::Handle::current();
            let result = handle.block_on(async move { provider.complete(messages, options).await });

            match result {
                Ok(resp) => {
                    let content = v8::String::new(scope, &resp.message.content).unwrap();
                    rv.set(content.into());
                }
                Err(e) => {
                    let err_msg = format!("llm.complete error: {}", e);
                    let msg = v8::String::new(scope, &err_msg).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                }
            }
        },
    );
    set_fn(scope, llm_obj, "complete", complete_tpl);

    // ── claw.llm.stream(messages, opts?) ──────────────────────────────────────
    let stream_tpl = v8::FunctionTemplate::new(
        scope,
        |scope: &mut v8::HandleScope<'_>,
         args: v8::FunctionCallbackArguments<'_>,
         mut rv: v8::ReturnValue<'_>| {
            let state = match unsafe { get_state(scope, args.this()) } {
                Some(s) => s,
                None => {
                    let msg = v8::String::new(scope, "bridge state unavailable").unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                    return;
                }
            };

            let provider = match &state.llm_provider {
                Some(p) => Arc::clone(p),
                None => {
                    let msg = v8::String::new(scope, "llm bridge: no provider configured").unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                    return;
                }
            };

            let messages_val = args.get(0);
            let messages = match llm_parse_messages(scope, messages_val) {
                Ok(m) => m,
                Err(e) => {
                    let msg = v8::String::new(scope, &e).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                    return;
                }
            };

            let opts_val = args.get(1);
            let options = match llm_parse_opts(scope, opts_val, provider.model_id()) {
                Ok(o) => o,
                Err(e) => {
                    let msg = v8::String::new(scope, &e).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                    return;
                }
            };

            let handle = tokio::runtime::Handle::current();
            let result = handle.block_on(async move {
                use futures::StreamExt;
                let mut stream = provider.complete_stream(messages, options).await?;
                let mut chunks: Vec<String> = Vec::new();
                while let Some(delta) = stream.next().await {
                    match delta {
                        Ok(d) => {
                            if let Some(content) = d.content {
                                if !content.is_empty() {
                                    chunks.push(content);
                                }
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(chunks)
            });

            match result {
                Ok(chunks) => {
                    let arr = v8::Array::new(scope, chunks.len() as i32);
                    for (i, chunk) in chunks.iter().enumerate() {
                        let s = v8::String::new(scope, chunk).unwrap();
                        arr.set_index(scope, i as u32, s.into());
                    }
                    rv.set(arr.into());
                }
                Err(e) => {
                    let err_msg = format!("llm.stream error: {}", e);
                    let msg = v8::String::new(scope, &err_msg).unwrap();
                    let exc = v8::Exception::error(scope, msg);
                    scope.throw_exception(exc);
                }
            }
        },
    );
    set_fn(scope, llm_obj, "stream", stream_tpl);

    let k = v8::String::new(scope, "llm").unwrap();
    claw_obj.set(scope, k.into(), llm_obj.into());
    Ok(())
}

/// Parse a V8 array of message objects into Vec<claw_provider::types::Message>.
fn llm_parse_messages(
    scope: &mut v8::HandleScope<'_>,
    val: v8::Local<v8::Value>,
) -> Result<Vec<claw_provider::types::Message>, String> {
    use claw_provider::types::{Message, Role};

    if !val.is_array() {
        return Err("messages must be an array".to_string());
    }
    let arr = v8::Local::<v8::Array>::try_from(val)
        .map_err(|_| "messages must be an array".to_string())?;
    let len = arr.length();
    let mut messages = Vec::with_capacity(len as usize);

    for i in 0..len {
        let entry = arr
            .get_index(scope, i)
            .ok_or_else(|| format!("messages[{}] is undefined", i))?;
        if !entry.is_object() {
            return Err(format!("messages[{}] must be an object", i));
        }
        let obj = v8::Local::<v8::Object>::try_from(entry)
            .map_err(|_| format!("messages[{}] must be an object", i))?;

        let role_key = v8::String::new(scope, "role").unwrap();
        let role_val = obj
            .get(scope, role_key.into())
            .ok_or_else(|| format!("messages[{}].role is missing", i))?;
        let role_str = role_val
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default();

        let content_key = v8::String::new(scope, "content").unwrap();
        let content_val = obj
            .get(scope, content_key.into())
            .ok_or_else(|| format!("messages[{}].content is missing", i))?;
        let content = content_val
            .to_string(scope)
            .map(|s| s.to_rust_string_lossy(scope))
            .unwrap_or_default();

        let role = match role_str.to_lowercase().as_str() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            "system" => Role::System,
            "tool" => Role::Tool,
            other => return Err(format!("messages[{}]: unknown role '{}'", i, other)),
        };

        messages.push(Message {
            role,
            content,
            tool_calls: None,
            tool_call_id: None,
        });
    }
    Ok(messages)
}

/// Parse a V8 opts object into claw_provider::types::Options.
fn llm_parse_opts(
    scope: &mut v8::HandleScope<'_>,
    val: v8::Local<v8::Value>,
    default_model: &str,
) -> Result<claw_provider::types::Options, String> {
    use claw_provider::types::Options;

    let model = if val.is_object() && !val.is_null() {
        let obj = v8::Local::<v8::Object>::try_from(val).ok();
        if let Some(obj) = obj {
            let key = v8::String::new(scope, "model").unwrap();
            obj.get(scope, key.into())
                .and_then(|v| v.to_string(scope))
                .map(|s| s.to_rust_string_lossy(scope))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| default_model.to_string())
        } else {
            default_model.to_string()
        }
    } else {
        default_model.to_string()
    };

    let mut options = Options::new(model);

    if val.is_object() && !val.is_null() {
        if let Ok(obj) = v8::Local::<v8::Object>::try_from(val) {
            // max_tokens
            let key = v8::String::new(scope, "max_tokens").unwrap();
            if let Some(v) = obj.get(scope, key.into()) {
                if v.is_number() {
                    let n = v.number_value(scope).unwrap_or(0.0) as u32;
                    if n > 0 {
                        options = options.with_max_tokens(n);
                    }
                }
            }
            // temperature
            let key = v8::String::new(scope, "temperature").unwrap();
            if let Some(v) = obj.get(scope, key.into()) {
                if v.is_number() {
                    let t = v.number_value(scope).unwrap_or(0.7) as f32;
                    options = options
                        .with_temperature(t)
                        .map_err(|e| format!("opts.temperature: {}", e))?;
                }
            }
        }
    }

    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{FsBridgeConfig, NetBridgeConfig};
    use claw_tools::types::PermissionSet;

    fn make_state() -> BridgeState {
        BridgeState::new(
            FsBridgeConfig::default(),
            NetBridgeConfig::default(),
            None,
            PermissionSet::minimal(),
            "test-agent".to_string(),
            None,
            None,
        )
    }

    // ── fs_validate_path ──────────────────────────────────────────────────────

    #[test]
    fn test_fs_validate_no_allowed_paths() {
        let state = make_state();
        let result = fs_validate_path(&state.fs_config, "/tmp/foo.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no filesystem access allowed"));
    }

    #[test]
    fn test_fs_validate_allowed_path() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().to_path_buf();
        let mut config = FsBridgeConfig::default();
        config.allowed_paths.insert(path.clone());
        config.base_dir = path.clone();

        // Create a file inside tmp.
        let file = path.join("test.txt");
        std::fs::write(&file, "hello").unwrap();

        let result = fs_validate_path(&config, file.to_str().unwrap());
        assert!(result.is_ok());
    }

    #[test]
    fn test_fs_validate_traversal_denied() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = tmp.path().join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        // Allowed: inner dir only.
        let mut config = FsBridgeConfig::default();
        config.allowed_paths.insert(inner.clone());
        config.base_dir = inner.clone();

        // Create a file outside.
        let outside = tmp.path().join("secret.txt");
        std::fs::write(&outside, "secret").unwrap();

        let result = fs_validate_path(&config, outside.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Permission denied"));
    }

    // ── net_validate_url ──────────────────────────────────────────────────────

    #[test]
    fn test_net_validate_no_domains() {
        let config = NetBridgeConfig::default();
        let result = net_validate_url(&config, "https://example.com/path");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no network access allowed"));
    }

    #[test]
    fn test_net_validate_allowed_domain() {
        let config = NetBridgeConfig::with_domains(vec!["example.com".to_string()]);
        let result = net_validate_url(&config, "https://example.com/path");
        assert!(result.is_ok());
    }

    #[test]
    fn test_net_validate_subdomain_allowed() {
        let config = NetBridgeConfig::with_domains(vec!["example.com".to_string()]);
        let result = net_validate_url(&config, "https://api.example.com/path");
        assert!(result.is_ok());
    }

    #[test]
    fn test_net_validate_denied_domain() {
        let config = NetBridgeConfig::with_domains(vec!["example.com".to_string()]);
        let result = net_validate_url(&config, "https://evil.com/path");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not in the allowlist"));
    }

    #[test]
    fn test_net_validate_loopback_denied_by_default() {
        let config = NetBridgeConfig::with_domains(vec!["example.com".to_string()]);
        let result = net_validate_url(&config, "http://localhost:80/path");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("loopback access not allowed"));
    }

    #[test]
    fn test_net_validate_port_not_allowed() {
        let config = NetBridgeConfig::with_domains(vec!["example.com".to_string()]);
        let result = net_validate_url(&config, "http://example.com:8080/path");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("port 8080 is not allowed"));
    }

    #[test]
    fn test_net_validate_invalid_scheme() {
        let config = NetBridgeConfig::with_domains(vec!["example.com".to_string()]);
        let result = net_validate_url(&config, "ftp://example.com/file");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported URL scheme"));
    }

    // ── BridgeState ───────────────────────────────────────────────────────────

    #[test]
    fn test_bridge_state_new() {
        let state = make_state();
        assert_eq!(state.agent_id, "test-agent");
        assert!(state.tool_registry.is_none());
        assert!(state.event_rx.is_none());
        assert!(state.orchestrator.is_none());
    }

    #[test]
    fn test_bridge_state_with_event_bus_creates_rx() {
        use std::sync::Arc;
        let bus = Arc::new(EventBus::new());
        let state = BridgeState::new(
            FsBridgeConfig::default(),
            NetBridgeConfig::default(),
            None,
            PermissionSet::minimal(),
            "agent-1".to_string(),
            Some(bus),
            None,
        );
        assert!(state.event_rx.is_some());
    }

    // ── event_to_parts ────────────────────────────────────────────────────────

    #[test]
    fn test_event_to_parts_shutdown() {
        let (t, d) = event_to_parts(&Event::Shutdown);
        assert_eq!(t, "shutdown");
        assert!(d.is_object());
    }

    #[test]
    fn test_event_to_parts_custom() {
        let event = Event::Custom {
            event_type: "my_event".to_string(),
            data: json!({"key": "value"}),
        };
        let (t, d) = event_to_parts(&event);
        assert_eq!(t, "my_event");
        assert_eq!(d["key"], "value");
    }

    // ── memory_item_id ────────────────────────────────────────────────────────

    #[test]
    fn test_memory_item_id_format() {
        let id = memory_item_id("agent-42", "my_key");
        assert_eq!(id.0, "agent-42::my_key");
    }
}
