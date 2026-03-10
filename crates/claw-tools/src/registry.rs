use dashmap::DashMap;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::{
    audit::{AuditEvent, AuditLogWriterHandle},
    error::RegistryError,
    sandbox::SandboxApplier,
    traits::{Tool, ToolEventPublisher},
    types::{LogEntry, RegistryExecutionMode, SubprocessPolicy, ToolContext, ToolError, ToolMeta, ToolResult, LoadedToolMeta},
};

/// Abstraction over a stored power-key hash that can verify a plaintext key.
///
/// This trait lets `ToolRegistry::enter_power_mode()` accept any hash type
/// (e.g. `claw_pal::security::PowerKeyHash`) without creating a hard
/// dependency on `claw-pal`, which would introduce a circular dependency.
///
/// # Safety contract
///
/// Implementations MUST use a constant-time comparison to prevent timing
/// attacks.  `claw_pal::security::PowerKeyHash` uses Argon2id which
/// satisfies this requirement.
pub trait PowerKeyVerify: Send + Sync {
    /// Return `true` iff `candidate` matches the stored hash.
    fn verify(&self, candidate: &str) -> bool;
}

/// RAII guard for Power Mode sessions in [`ToolRegistry`].
///
/// Returned by [`ToolRegistry::enter_power_mode`]. While this guard is alive the
/// registry operates in Power Mode (all permission checks bypassed). When the guard
/// is dropped the registry reverts to Safe Mode automatically.
///
/// # Process-restart semantics (ADR-003)
///
/// Per ADR-003, Power → Safe is not a valid software transition — it normally
/// requires a process restart. Within the `ToolRegistry` model the guard plays
/// the role of a process-lifetime token: the caller holds it for the duration of
/// the privileged session and dropping it (or letting it fall out of scope) is the
/// sole approved mechanism for ending that session, mirroring what a process restart
/// would achieve in a full deployment.
///
/// # Security note
///
/// Keep the guard in a tightly-scoped block. Do **not** leak it via `Box::leak` or
/// store it in an `Arc` that outlives the intended session — that defeats the reset.
pub struct PowerModeGuard {
    mode: Arc<std::sync::RwLock<RegistryExecutionMode>>,
}

impl Drop for PowerModeGuard {
    /// Revert the registry to Safe Mode when the guard is dropped.
    ///
    /// The reset happens even on panics, ensuring the registry never permanently
    /// remains in Power Mode due to an error in caller code.
    fn drop(&mut self) {
        // If the lock is poisoned a previous panic already unwound the stack;
        // the mode value is unreliable — skip the reset rather than double-panicking.
        if let Ok(mut m) = self.mode.write() {
            *m = RegistryExecutionMode::Safe;
        }
        tracing::info!("ToolRegistry: PowerModeGuard dropped — global mode reset to SAFE");
    }
}

/// Thread-safe tool registry with permission checking and timeout execution.
pub struct ToolRegistry {
    tools: DashMap<String, Arc<dyn Tool>>,
    audit_log: RwLock<BTreeMap<u64, LogEntry>>, // key → entry (ordered by ID)
    max_audit_entries: usize,
    /// Per-instance monotonic counter for audit log entry IDs.
    /// Instance-level field (instead of a global static) ensures each registry
    /// maintains its own independent counter, preventing cross-instance ID leakage.
    audit_counter: AtomicU64,
    /// Hot reload processor (if enabled)
    hot_reload: tokio::sync::RwLock<Option<crate::hot_reload::HotReloadProcessor>>,
    /// Loaded script tools metadata
    script_tools: DashMap<String, LoadedToolMeta>,
    /// Optional event publisher for tool lifecycle events (TASK-28)
    event_publisher: Option<Arc<dyn ToolEventPublisher>>,
    /// Optional persistent audit log writer (HMAC-signed, with async file I/O).
    ///
    /// When `Some`, every tool call and mode-switch is forwarded to the background
    /// `AuditLogWriter` task in addition to the in-memory `BTreeMap` log.
    audit_writer: Option<AuditLogWriterHandle>,
    /// Optional OS-level sandbox applier (G-2 fix).
    ///
    /// When `Some` and the registry is in `Safe` mode, `apply_safe_mode()` is called
    /// exactly once (on the first `execute()` call) via `sandbox_state`.  After
    /// successful application the OS-level restrictions complement the Rust-layer
    /// permission checks with kernel-enforced isolation (seccomp-bpf on Linux,
    /// `sandbox_init` on macOS).
    ///
    /// In the main kernel process use `NoopSandboxApplier` (or leave `None`) to
    /// avoid sandboxing the daemon itself.  Inject a real implementation in agent
    /// subprocesses that are dedicated to tool execution.
    sandbox_applier: Option<Arc<dyn SandboxApplier>>,
    /// Tracks the result of the one-shot OS sandbox application.
    ///
    /// `OnceLock` guarantees that `sandbox_applier.apply_safe_mode()` is invoked
    /// at most once per registry instance, even under concurrent `execute()` calls.
    /// A stored `Err(String)` causes all subsequent `execute()` calls to fail-closed.
    sandbox_state: OnceLock<Result<(), String>>,
    /// Global execution mode — cannot be overridden by per-call ToolContext.
    ///
    /// When `Safe` (default), all permission checks are enforced regardless of
    /// what `ToolContext::execution_mode` the caller specifies.  The only way to
    /// switch to `Power` mode is via `ToolRegistry::enter_power_mode()`, which
    /// requires a verified power-key hash.
    global_mode: Arc<std::sync::RwLock<RegistryExecutionMode>>,
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: DashMap::new(),
            audit_log: RwLock::new(BTreeMap::new()),
            max_audit_entries: 10_000,
            audit_counter: AtomicU64::new(1),
            hot_reload: tokio::sync::RwLock::new(None),
            script_tools: DashMap::new(),
            event_publisher: None,
            audit_writer: None,
            sandbox_applier: None,
            sandbox_state: OnceLock::new(),
            global_mode: Arc::new(std::sync::RwLock::new(RegistryExecutionMode::Safe)),
        }
    }

    /// Set the maximum number of audit log entries retained in memory.
    ///
    /// When the log exceeds this limit, the oldest 10 % of entries are evicted.
    pub fn with_max_audit_entries(mut self, max: usize) -> Self {
        self.max_audit_entries = max;
        self
    }

    /// Attach an event publisher for tool lifecycle events.
    ///
    /// When set, the registry will publish events on tool calls, results,
    /// registrations, and unregistrations. This enables Layer 1 (claw-runtime)
    /// to observe tool activity via the EventBus without circular dependencies.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claw_tools::{ToolRegistry, NoopToolEventPublisher};
    /// use std::sync::Arc;
    ///
    /// let registry = ToolRegistry::new()
    ///     .with_event_publisher(NoopToolEventPublisher::new());
    /// ```
    pub fn with_event_publisher(mut self, p: Arc<dyn ToolEventPublisher>) -> Self {
        self.event_publisher = Some(p);
        self
    }

    /// Attach a persistent audit log writer for tamper-evident, HMAC-signed logging.
    ///
    /// When set, every `execute()` call writes `AuditEvent::ToolCall` +
    /// `AuditEvent::ToolResult` to the background `AuditLogWriter` task, and
    /// `enter_power_mode()` writes an `AuditEvent::ModeSwitch` record.
    ///
    /// The existing in-memory `BTreeMap` log is retained for backward compatibility
    /// with `recent_log()`.
    pub fn with_audit_writer(mut self, handle: AuditLogWriterHandle) -> Self {
        self.audit_writer = Some(handle);
        self
    }

    /// Inject an OS-level sandbox applier (G-2 fix).
    ///
    /// When set, the first `execute()` call in `Safe` mode will invoke
    /// `applier.apply_safe_mode()` exactly once.  The OS-level restrictions then
    /// complement the Rust-layer glob-based permission checks for the lifetime of
    /// this registry instance.
    ///
    /// # When to use
    ///
    /// - **Agent subprocesses** (dedicated to tool execution): inject a real
    ///   `SandboxApplier` backed by `claw_pal::linux::LinuxSandbox` or
    ///   `claw_pal::macos::MacOSSandbox` for kernel-enforced isolation.
    /// - **Main kernel process** (runs the daemon alongside channels, event bus,
    ///   etc.): use [`NoopSandboxApplier`][crate::sandbox::NoopSandboxApplier] or
    ///   omit this call — sandboxing the whole daemon would break network and FS
    ///   access for other components.
    ///
    /// # Fail-closed behaviour
    ///
    /// If `apply_safe_mode()` returns an error, all subsequent `execute()` calls
    /// return `RegistryError::ExecutionFailed` immediately.  This prevents tools
    /// from running without OS-level protection when protection was explicitly
    /// requested.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claw_tools::{ToolRegistry, sandbox::{SandboxApplier, NoopSandboxApplier}};
    /// use std::sync::Arc;
    ///
    /// // In production agent subprocess: replace with a real platform applier.
    /// let registry = ToolRegistry::new()
    ///     .with_sandbox_applier(NoopSandboxApplier::new());
    /// ```
    pub fn with_sandbox_applier(mut self, applier: Arc<dyn SandboxApplier>) -> Self {
        self.sandbox_applier = Some(applier);
        self
    }

    /// Generate a unique monotonic audit ID using a per-instance AtomicU64 counter.
    ///
    /// This approach eliminates clock skew risk (NTP sync, manual time changes)
    /// by using a purely monotonic counter instead of timestamp-based IDs.
    /// Uses SeqCst ordering for maximum thread safety.
    fn generate_audit_id(&self) -> u64 {
        self.audit_counter.fetch_add(1, Ordering::SeqCst)
    }

    /// Register a tool. Returns `RegistryError::AlreadyExists` if already registered.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claw_tools::{ToolRegistry, Tool, ToolSchema, PermissionSet, ToolContext, ToolResult};
    /// use async_trait::async_trait;
    ///
    /// struct MyTool;
    ///
    /// #[async_trait]
    /// impl Tool for MyTool {
    ///     fn name(&self) -> &str { "my_tool" }
    ///     fn description(&self) -> &str { "A useful tool" }
    ///     fn schema(&self) -> &ToolSchema {
    ///         static SCHEMA: std::sync::OnceLock<ToolSchema> = std::sync::OnceLock::new();
    ///         SCHEMA.get_or_init(|| ToolSchema::new("my_tool", "A useful tool", serde_json::json!({})))
    ///     }
    ///     fn permissions(&self) -> &PermissionSet {
    ///         static PERMS: std::sync::OnceLock<PermissionSet> = std::sync::OnceLock::new();
    ///         PERMS.get_or_init(PermissionSet::minimal)
    ///     }
    ///     async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
    ///         ToolResult::ok(args, 0)
    ///     }
    /// }
    ///
    /// let registry = ToolRegistry::new();
    /// registry.register(Box::new(MyTool)).expect("registration succeeds");
    /// assert_eq!(registry.tool_count(), 1);
    /// assert!(registry.tool_names().contains(&"my_tool".to_string()));
    /// ```
    pub fn register(&self, tool: Box<dyn Tool>) -> Result<(), RegistryError> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            return Err(RegistryError::AlreadyExists(name));
        }
        self.tools.insert(name.clone(), Arc::from(tool));
        if let Some(publisher) = &self.event_publisher {
            publisher.publish_tool_registered(&name, "native");
        }
        Ok(())
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).map(|t| Arc::clone(&*t))
    }

    /// Unregister a tool. Returns `RegistryError::ToolNotFound` if not registered.
    pub fn unregister(&self, name: &str) -> Result<(), RegistryError> {
        if self.tools.remove(name).is_none() {
            return Err(RegistryError::ToolNotFound(name.to_string()));
        }
        if let Some(publisher) = &self.event_publisher {
            publisher.publish_tool_unregistered(name);
        }
        Ok(())
    }

    /// Update (or register) a tool.
    /// If the tool exists, it is replaced; otherwise it is registered.
    pub fn update(&self, name: &str, tool: Arc<dyn Tool>) -> Result<(), RegistryError> {
        self.tools.insert(name.to_string(), tool);
        Ok(())
    }

    /// Get tool metadata (schema + permissions) without executing.
    pub fn tool_meta(&self, name: &str) -> Option<ToolMeta> {
        self.tools.get(name).map(|t| ToolMeta {
            schema: t.schema().clone(),
            permissions: t.permissions().clone(),
            timeout: t.timeout(),
            source: crate::types::ToolSource::Native,
        })
    }

    /// List all registered tool names.
    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|e| e.key().clone()).collect()
    }

    /// Count of registered tools.
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    /// Return the current global execution mode.
    pub fn global_mode(&self) -> RegistryExecutionMode {
        *self.global_mode.read().expect("global_mode lock poisoned")
    }

    /// Enter Power Mode by verifying a power-key against a pre-computed Argon2 hash.
    ///
    /// # Security
    ///
    /// The `stored_hash` should have been produced by `claw_pal::security::PowerKeyHash::new()`
    /// (Argon2id, random salt).  Verification is performed by calling `stored_hash.verify()`.
    ///
    /// Once Power Mode is entered the registry bypasses all permission checks. The
    /// returned [`PowerModeGuard`] must be kept alive for as long as the privileged
    /// session should last; dropping it atomically resets the registry to Safe Mode
    /// (process-restart semantics per ADR-003).
    ///
    /// # Errors
    ///
    /// Returns `RegistryError::ExecutionFailed` if the key does not match.
    pub fn enter_power_mode<H>(&self, power_key: &str, stored_hash: &H) -> Result<PowerModeGuard, RegistryError>
    where
        H: PowerKeyVerify,
    {
        if !stored_hash.verify(power_key) {
            return Err(RegistryError::ExecutionFailed(
                "invalid power key: cannot enter Power Mode".to_string(),
            ));
        }
        *self.global_mode.write().expect("global_mode lock poisoned") = RegistryExecutionMode::Power;
        tracing::warn!(
            "ToolRegistry: global mode switched to POWER — permission checks bypassed"
        );
        // Record Safe→Power mode transition in the persistent audit log.
        if let Some(w) = &self.audit_writer {
            w.send_blocking(AuditEvent::ModeSwitch {
                timestamp_ms: Self::now_ms(),
                agent_id: "system".to_string(),
                from_mode: "safe".to_string(),
                to_mode: "power".to_string(),
                reason: "power_key_verified".to_string(),
            });
        }
        Ok(PowerModeGuard {
            mode: Arc::clone(&self.global_mode),
        })
    }

    /// Execute a tool with permission checking, timeout, and audit logging.
    ///
    /// This is the main entry point for tool execution. It performs the following steps:
    /// 1. Looks up the tool by name
    /// 2. Validates permissions (filesystem, network, subprocess)
    /// 3. Executes the tool with a timeout using `JoinSet` for cancellation safety
    /// 4. Records the execution in the audit log
    ///
    /// # Permission Checking
    ///
    /// - **Subprocess**: If tool requires `SubprocessPolicy::Allowed` but context grants `Denied`,
    ///   returns `RegistryError::PermissionDenied`
    /// - **Filesystem**: Tool's read/write paths must be a subset of context's allowed paths
    /// - **Network**: Tool's allowed domains must be a subset of context's allowed domains
    ///
    /// # Timeout Handling
    ///
    /// Uses `tool.timeout()` (default 30s) wrapped in `tokio::time::timeout()`.
    /// When timeout occurs, the task is cancelled via `CancellationToken` and
    /// a `ToolResult` with `ToolErrorCode::Timeout` is returned.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the tool to execute
    /// * `args` - JSON value containing the tool arguments
    /// * `ctx` - Tool context containing agent ID and permissions
    ///
    /// # Returns
    ///
    /// - `Ok(ToolResult)` - Tool executed (check `ToolResult::success` for outcome)
    /// - `Err(RegistryError)` - Tool not found or permission denied
    ///
    /// # Example
    ///
    /// ```
    /// use claw_tools::{registry::ToolRegistry, types::{ToolContext, PermissionSet}};
    /// use serde_json::json;
    ///
    /// async fn example() {
    ///     let registry = ToolRegistry::new();
    ///     // ... register tools ...
    ///
    ///     let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
    ///     let result = registry.execute("echo", json!({"message": "hello"}), ctx).await;
    ///
    ///     match result {
    ///         Ok(tool_result) if tool_result.success => {
    ///             println!("Success: {:?}", tool_result.output);
    ///         }
    ///         Ok(tool_result) => {
    ///             println!("Tool failed: {:?}", tool_result.error);
    ///         }
    ///         Err(e) => {
    ///             println!("Execution error: {}", e);
    ///         }
    ///     }
    /// }
    /// ```
    pub async fn execute(
        &self,
        name: &str,
        args: serde_json::Value,
        ctx: ToolContext,
    ) -> Result<ToolResult, RegistryError> {
        // 1. Find tool
        let tool = self
            .tools
            .get(name)
            .map(|e| Arc::clone(&*e))
            .ok_or_else(|| RegistryError::ToolNotFound(name.to_string()))?;

        // 2. Resolve effective execution mode.
        //
        // Security invariant: the *global* Safe mode cannot be overridden by
        // a caller that sets ctx.execution_mode = Power.  Only a verified
        // `enter_power_mode()` call can promote the registry to Power mode.
        //
        // Effective mode = max(global_mode, ctx.execution_mode)
        // where Safe < Power.
        let global = self.global_mode();
        let effective_mode = if global == RegistryExecutionMode::Power {
            RegistryExecutionMode::Power
        } else {
            // Global is Safe: ctx Power is silently clamped to Safe.
            if ctx.execution_mode == RegistryExecutionMode::Power {
                tracing::warn!(
                    tool = %name,
                    agent = %ctx.agent_id,
                    "caller requested Power mode but registry is globally locked to Safe — \
                     request ignored"
                );
            }
            RegistryExecutionMode::Safe
        };

        // 2.5. G-2 fix: apply OS-level sandbox on the first Safe-mode execution.
        //
        // `sandbox_state` is an `OnceLock<Result<(), String>>` — `get_or_init` is
        // called on every execute() but only runs the closure once, even under
        // concurrent callers (std::sync::OnceLock is thread-safe by spec).
        //
        // Fail-closed: if the sandbox couldn't be applied we refuse ALL tool calls.
        // This prevents a "partial sandbox" scenario where some tools run protected
        // and others run unprotected because of an apply() error.
        if effective_mode == RegistryExecutionMode::Safe {
            if let Some(ref applier) = self.sandbox_applier {
                let applier_ref = Arc::clone(applier);
                let state = self.sandbox_state.get_or_init(|| {
                    let result = applier_ref.apply_safe_mode();
                    match &result {
                        Ok(()) => tracing::info!(
                            "ToolRegistry: OS-level sandbox applied to process (Safe mode, G-2)"
                        ),
                        Err(ref msg) => tracing::error!(
                            error = %msg,
                            "ToolRegistry: OS sandbox application failed — \
                             all subsequent tool executions will be refused (fail-closed)"
                        ),
                    }
                    result
                });
                if let Err(ref msg) = *state {
                    return Err(RegistryError::ExecutionFailed(format!(
                        "OS sandbox apply failed — tool execution refused for safety: {msg}"
                    )));
                }
            }
        }

        // Skip all permission checks when in Power mode.
        if effective_mode == RegistryExecutionMode::Safe {
            // 2a. Subprocess policy
            if tool.permissions().subprocess == SubprocessPolicy::Allowed
                && ctx.permissions.subprocess == SubprocessPolicy::Denied
            {
                return Err(RegistryError::PermissionDenied {
                    tool: name.to_string(),
                    permission: "subprocess".to_string(),
                });
            }

            // 2b. Filesystem permission check (glob-aware, GAP-F4-01)
            {
                let tool_fs = &tool.permissions().filesystem;
                let ctx_fs = &ctx.permissions.filesystem;
                if !tool_fs.read_paths.is_empty()
                    && !tool_fs
                        .read_paths
                        .iter()
                        .all(|p| fs_path_covered(p, &ctx_fs.read_paths))
                {
                    return Err(RegistryError::PermissionDenied {
                        tool: name.to_string(),
                        permission: "filesystem:read".to_string(),
                    });
                }
                if !tool_fs.write_paths.is_empty()
                    && !tool_fs
                        .write_paths
                        .iter()
                        .all(|p| fs_path_covered(p, &ctx_fs.write_paths))
                {
                    return Err(RegistryError::PermissionDenied {
                        tool: name.to_string(),
                        permission: "filesystem:write".to_string(),
                    });
                }
            }

            // 2c. Network permission check
            {
                let tool_net = &tool.permissions().network;
                let ctx_net = &ctx.permissions.network;
                if !tool_net.allowed_domains.is_empty()
                    && !tool_net.allowed_domains.is_subset(&ctx_net.allowed_domains)
                {
                    return Err(RegistryError::PermissionDenied {
                        tool: name.to_string(),
                        permission: "network".to_string(),
                    });
                }
            }
        }

        // 3. Execute with timeout and cancellation support using JoinSet (RES-001)
        // FIX-20: treat zero-duration timeout as "no limit configured" and fall back to
        // a safe 30-second default so tools cannot run forever due to misconfiguration.
        const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
        let raw_timeout = tool.timeout();
        let tool_timeout = if raw_timeout.is_zero() {
            tracing::warn!(
                tool = %name,
                "工具报告零超时；使用 30 秒回退值以防无限阻塞"
            );
            DEFAULT_TIMEOUT
        } else {
            raw_timeout
        };
        let start = Instant::now();
        let cancellation_token = CancellationToken::new();

        // TASK-28: generate a unique call_id for event correlation
        let call_id = format!("call-{}", self.audit_counter.fetch_add(1, Ordering::SeqCst));

        // TASK-28: publish ToolCalled event before execution (debug-level tracing supplement)
        if let Some(publisher) = &self.event_publisher {
            publisher.publish_tool_called(&ctx.agent_id, name, &call_id);
        }
        tracing::debug!(tool = %name, call_id = %call_id, agent = %ctx.agent_id, "tool called");

        // G-1 fix: forward ToolCall event to the persistent AuditLogWriter (HMAC path).
        if let Some(w) = &self.audit_writer {
            w.send_blocking(AuditEvent::ToolCall {
                timestamp_ms: Self::now_ms(),
                agent_id: ctx.agent_id.clone(),
                tool_name: name.to_string(),
                args: Some(args.clone()),
            });
        }

        // Create a JoinSet to manage the tool execution task
        let mut join_set = JoinSet::new();

        // Spawn the tool execution into the JoinSet
        let tool_clone = Arc::clone(&tool);
        let args_clone = args.clone();
        let ctx_clone = ctx.clone();
        let cancel_clone = cancellation_token.clone();

        join_set.spawn(async move {
            // Future that wraps tool execution with cancellation check
            tokio::select! {
                result = tool_clone.execute(args_clone, &ctx_clone) => {
                    result
                }
                _ = cancel_clone.cancelled() => {
                    // Cancelled by timeout - return timeout error
                    ToolResult::err(ToolError::timeout(), 0)
                }
            }
        });

        // Wait for completion with timeout using JoinSet
        let tool_result = match tokio::time::timeout(tool_timeout, join_set.join_next()).await {
            Ok(Some(Ok(result))) => {
                // Task completed successfully
                result
            }
            Ok(Some(Err(join_err))) => {
                // Task panicked or was cancelled
                if join_err.is_cancelled() || join_err.is_panic() {
                    ToolResult::err(
                        ToolError::internal(format!("Task error: {}", join_err)),
                        start.elapsed().as_millis() as u64,
                    )
                } else {
                    ToolResult::err(
                        ToolError::internal(format!("Task join error: {}", join_err)),
                        start.elapsed().as_millis() as u64,
                    )
                }
            }
            Ok(None) => {
                // JoinSet is empty (should not happen as we only spawn one task)
                ToolResult::err(
                    ToolError::internal("Task not spawned"),
                    start.elapsed().as_millis() as u64,
                )
            }
            Err(_) => {
                // Timeout occurred - cancel the token and shutdown all tasks in JoinSet
                cancellation_token.cancel();
                join_set.shutdown().await;

                let elapsed_ms = start.elapsed().as_millis() as u64;
                // TASK-28: publish ToolResult event (timeout = failure)
                if let Some(publisher) = &self.event_publisher {
                    publisher.publish_tool_result(&ctx.agent_id, name, &call_id, false);
                }
                tracing::debug!(tool = %name, call_id = %call_id, success = false, "tool result (timeout)");
                let entry = self.make_log_entry(&ctx.agent_id, name, false, elapsed_ms);
                self.insert_log(entry).await;
                // G-1 fix: write ToolResult (timeout) to persistent audit writer.
                if let Some(w) = &self.audit_writer {
                    w.send_blocking(AuditEvent::ToolResult {
                        timestamp_ms: Self::now_ms(),
                        agent_id: ctx.agent_id.clone(),
                        tool_name: name.to_string(),
                        success: false,
                        duration_ms: elapsed_ms,
                        error_code: Some("timeout".to_string()),
                    });
                }
                return Ok(ToolResult::err(ToolError::timeout(), elapsed_ms));
            }
        };

        let elapsed_ms = start.elapsed().as_millis() as u64;

        // TASK-28: publish ToolResult event after execution completes
        if let Some(publisher) = &self.event_publisher {
            publisher.publish_tool_result(&ctx.agent_id, name, &call_id, tool_result.success);
        }
        tracing::debug!(tool = %name, call_id = %call_id, success = %tool_result.success, "tool result");

        // 4. Audit log
        let entry = self.make_log_entry(&ctx.agent_id, name, tool_result.success, elapsed_ms);
        self.insert_log(entry).await;

        // G-1 fix: write ToolResult to persistent audit writer (HMAC path).
        if let Some(w) = &self.audit_writer {
            let error_code = tool_result.error.as_ref().map(|e| format!("{:?}", e.code));
            w.send_blocking(AuditEvent::ToolResult {
                timestamp_ms: Self::now_ms(),
                agent_id: ctx.agent_id.clone(),
                tool_name: name.to_string(),
                success: tool_result.success,
                duration_ms: elapsed_ms,
                error_code,
            });
        }

        Ok(tool_result)
    }

    /// Recent audit log entries (last N entries by timestamp).
    pub async fn recent_log(&self, n: usize) -> Vec<LogEntry> {
        let guard = self.audit_log.read().await;
        guard.values().rev().take(n).cloned().collect()
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    fn now_ms() -> u64 {
        // FIX-28: log a warning when the system clock is before the Unix epoch
        // (e.g., due to clock misconfiguration) instead of silently returning 0.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|e| {
                tracing::warn!("System clock is before the Unix epoch: {e}");
                std::time::Duration::ZERO
            })
            .as_millis() as u64
    }

    fn make_log_entry(
        &self,
        agent_id: &str,
        tool_name: &str,
        success: bool,
        duration_ms: u64,
    ) -> LogEntry {
        LogEntry {
            timestamp_ms: Self::now_ms(),
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            success,
            duration_ms,
        }
    }

    async fn insert_log(&self, entry: LogEntry) {
        // Use monotonic counter-based ID to eliminate clock skew risk.
        let key = self.generate_audit_id();

        let mut guard = self.audit_log.write().await;
        guard.insert(key, entry);

        // Evict the oldest 10 % when over capacity (smallest IDs are oldest).
        // FIX-26: collect keys to remove first, then batch-delete to avoid repeated
        // first_key_value() + remove() calls which are redundant O(log n) per entry.
        if guard.len() > self.max_audit_entries {
            let to_remove = (self.max_audit_entries / 10).max(1);
            let keys_to_remove: Vec<u64> = guard.keys().take(to_remove).copied().collect();
            for key in keys_to_remove {
                guard.remove(&key);
            }
        }
    }
}

// =============================================================================
// Hot-loading API
// =============================================================================

impl ToolRegistry {
    /// Load a tool from a script file.
    ///
    /// # Arguments
    /// * `path` - Path to the script file
    ///
    /// # Returns
    /// * `Ok(ToolMeta)` - Tool metadata loaded successfully
    /// * `Err(LoadError)` - Loading failed
    ///
    /// # Note
    /// This loads metadata only. Actual script compilation requires
    /// ScriptEngine integration (application layer responsibility).
    pub async fn load_from_script(&self, path: &std::path::Path) -> Result<ToolMeta, crate::types::LoadError> {
        use crate::types::{ScriptLanguage, ToolSchema, PermissionSet, ToolSource};
        use std::time::Duration;

        if !path.exists() {
            return Err(crate::types::LoadError::IoError(format!(
                "File not found: {}",
                path.display()
            )));
        }

        let _content = tokio::fs::read_to_string(path)
            .await
            .map_err(|e| crate::types::LoadError::IoError(e.to_string()))?;

        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let language = match extension {
            "lua" => ScriptLanguage::Lua,
            "js" | "ts" => ScriptLanguage::TypeScript,
            "py" => ScriptLanguage::Python,
            _ => {
                return Err(crate::types::LoadError::ParseError {
                    path: path.display().to_string(),
                    message: format!("Unknown script extension: {}", extension),
                });
            }
        };

        let tool_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| crate::types::LoadError::ParseError {
                path: path.display().to_string(),
                message: "Invalid file name".to_string(),
            })?;

        let meta = ToolMeta {
            schema: ToolSchema::new(
                tool_name,
                format!("Script tool from {}", path.display()),
                serde_json::json!({"type": "object", "properties": {}}),
            ),
            permissions: PermissionSet::minimal(),
            timeout: Duration::from_secs(30),
            source: ToolSource::Script {
                path: path.to_path_buf(),
                language,
            },
        };

        self.script_tools.insert(
            tool_name.to_string(),
            LoadedToolMeta {
                name: tool_name.to_string(),
                source: meta.source.clone(),
                loaded_at: std::time::SystemTime::now(),
            },
        );

        Ok(meta)
    }

    /// Load all tools from a directory.
    pub async fn load_from_directory(
        &self,
        path: &std::path::Path,
    ) -> Result<Vec<ToolMeta>, crate::types::LoadError> {
        let mut results = Vec::new();

        let mut entries = tokio::fs::read_dir(path)
            .await
            .map_err(|e| crate::types::LoadError::IoError(e.to_string()))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| crate::types::LoadError::IoError(e.to_string()))?
        {
            let path = entry.path();
            if path.is_file() {
                match self.load_from_script(&path).await {
                    Ok(meta) => results.push(meta),
                    Err(e) => {
                        tracing::warn!("Failed to load tool from {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(results)
    }

    /// Unload a script tool.
    pub fn unload(&self, name: &str) -> Result<(), RegistryError> {
        self.script_tools.remove(name);
        self.unregister(name)
    }

    /// Enable hot-loading for the registry.
    pub async fn enable_hot_loading(
        &self,
        config: crate::types::HotLoadingConfig,
    ) -> Result<(), crate::types::WatchError> {
        config.validate().map_err(crate::types::WatchError::InvalidConfig)?;

        // Placeholder: actual implementation requires ScriptEngine integration
        tracing::info!("Hot-loading enabled with config: {:?}", config);

        Ok(())
    }

    /// Disable hot-loading.
    pub async fn disable_hot_loading(&self) {
        let mut guard = self.hot_reload.write().await;
        *guard = None;
        tracing::info!("Hot-loading disabled");
    }
}

// ─── Glob-aware filesystem permission helper ────────────────────────────────

/// Check whether `tool_path` is covered by any entry in `ctx_patterns`.
///
/// Matching strategy (first match wins):
/// 1. Exact string match — preserves backward-compat with existing configs.
/// 2. Glob match — each `ctx_patterns` entry is compiled as a `glob::Pattern`
///    and matched against `tool_path` as a literal filesystem path.
///    `*` does NOT cross path separators; `**` does (Unix glob semantics).
///    Invalid pattern strings (malformed globs) are silently skipped.
fn fs_path_covered(tool_path: &str, ctx_patterns: &std::collections::HashSet<String>) -> bool {
    let path = std::path::Path::new(tool_path);
    let opts = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    };
    ctx_patterns.iter().any(|pat| {
        // Fast path: exact string match
        if pat == tool_path {
            return true;
        }
        // Glob match with Unix path-separator semantics.
        glob::Pattern::new(pat)
            .ok()
            .map(|p| p.matches_path_with(path, opts))
            .unwrap_or(false)
    })
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "builtins")]
impl ToolRegistry {
    /// Create a new registry pre-loaded with all safe built-in tools.
    ///
    /// Registered by default: `web_fetch`, `file_read`, `file_write`.
    /// NOT included: `ShellExecTool` (security risk — register explicitly if needed).
    ///
    /// # Panics
    ///
    /// Panics if built-in registration fails (should never happen under normal conditions).
    pub fn with_builtins() -> Self {
        let reg = Self::new();
        crate::builtins::register_all_builtins(&reg)
            .expect("built-in tools should always register successfully");
        reg
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        FsPermissions, NetworkPermissions, PermissionSet, ToolContext, ToolResult, ToolSchema,
    };
    use async_trait::async_trait;
    use std::time::Duration;

    // ── Mock tools ────────────────────────────────────────────────────────────

    struct EchoTool {
        schema: ToolSchema,
        perms: PermissionSet,
    }

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echo"
        }
        fn schema(&self) -> &ToolSchema {
            &self.schema
        }
        fn permissions(&self) -> &PermissionSet {
            &self.perms
        }
        async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::ok(args, 0)
        }
    }

    struct SlowTool {
        schema: ToolSchema,
        perms: PermissionSet,
    }

    #[async_trait]
    impl Tool for SlowTool {
        fn name(&self) -> &str {
            "slow"
        }
        fn description(&self) -> &str {
            "Slow tool"
        }
        fn schema(&self) -> &ToolSchema {
            &self.schema
        }
        fn permissions(&self) -> &PermissionSet {
            &self.perms
        }
        fn timeout(&self) -> Duration {
            Duration::from_millis(1) // very short timeout
        }
        async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
            tokio::time::sleep(Duration::from_secs(60)).await;
            ToolResult::ok(serde_json::json!("done"), 0)
        }
    }

    fn make_echo_tool() -> Box<dyn Tool> {
        Box::new(EchoTool {
            schema: ToolSchema::new("echo", "Echo", serde_json::json!({})),
            perms: PermissionSet::minimal(),
        })
    }

    fn make_slow_tool() -> Box<dyn Tool> {
        Box::new(SlowTool {
            schema: ToolSchema::new("slow", "Slow tool", serde_json::json!({})),
            perms: PermissionSet::minimal(),
        })
    }

    fn default_ctx() -> ToolContext {
        ToolContext::new("agent-1", PermissionSet::minimal())
    }

    // ── Tests ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_registry_new() {
        let reg = ToolRegistry::new();
        assert_eq!(reg.tool_count(), 0);
        assert!(reg.tool_names().is_empty());
    }

    #[test]
    fn test_registry_register_and_count() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        assert_eq!(reg.tool_count(), 1);
    }

    #[test]
    fn test_registry_register_duplicate_fails() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        let err = reg.register(make_echo_tool()).unwrap_err();
        assert!(matches!(err, RegistryError::AlreadyExists(_)));
    }

    #[test]
    fn test_registry_unregister() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        reg.unregister("echo").unwrap();
        assert_eq!(reg.tool_count(), 0);
    }

    #[test]
    fn test_registry_unregister_nonexistent_fails() {
        let reg = ToolRegistry::new();
        let err = reg.unregister("nonexistent").unwrap_err();
        assert!(matches!(err, RegistryError::ToolNotFound(_)));
    }

    #[test]
    fn test_registry_tool_names() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        let names = reg.tool_names();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"echo".to_string()));
    }

    #[test]
    fn test_registry_tool_meta() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        let meta = reg.tool_meta("echo").expect("should have meta");
        assert_eq!(meta.schema.name, "echo");
        assert_eq!(meta.timeout, Duration::from_secs(30));
        assert!(matches!(meta.source, crate::types::ToolSource::Native))
    }

    #[tokio::test]
    async fn test_registry_execute_echo_tool() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        let args = serde_json::json!({"msg": "hello"});
        let result = reg
            .execute("echo", args.clone(), default_ctx())
            .await
            .unwrap();
        assert!(result.success);
        assert_eq!(result.output.as_ref().unwrap(), &args);
    }

    #[tokio::test]
    async fn test_registry_execute_nonexistent_tool_fails() {
        let reg = ToolRegistry::new();
        let err = reg
            .execute("ghost", serde_json::json!({}), default_ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, RegistryError::ToolNotFound(_)));
    }

    #[tokio::test]
    async fn test_registry_execute_timeout() {
        let reg = ToolRegistry::new();
        reg.register(make_slow_tool()).unwrap();
        let result = reg
            .execute("slow", serde_json::json!({}), default_ctx())
            .await
            .unwrap();
        // Should return a ToolResult with Timeout error (not a RegistryError)
        assert!(!result.success);
        let err = result.error.as_ref().expect("should have error");
        assert_eq!(err.code, crate::types::ToolErrorCode::Timeout);
    }

    // ── FS permission tests ───────────────────────────────────────────────────

    /// A mock tool whose `name()` delegates to `schema.name`, allowing tests to
    /// create tools with distinct names without defining a new struct each time.
    struct NamedTool {
        schema: ToolSchema,
        perms: PermissionSet,
    }

    #[async_trait]
    impl Tool for NamedTool {
        fn name(&self) -> &str {
            &self.schema.name
        }
        fn description(&self) -> &str {
            &self.schema.description
        }
        fn schema(&self) -> &ToolSchema {
            &self.schema
        }
        fn permissions(&self) -> &PermissionSet {
            &self.perms
        }
        async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
            ToolResult::ok(args, 0)
        }
    }

    fn make_fs_read_tool(path: &str) -> Box<dyn Tool> {
        Box::new(NamedTool {
            schema: ToolSchema::new("echo_fs_read", "FS read tool", serde_json::json!({})),
            perms: PermissionSet {
                filesystem: FsPermissions::read_only(vec![path.to_string()]),
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Denied,
            },
        })
    }

    fn make_fs_write_tool(path: &str) -> Box<dyn Tool> {
        Box::new(NamedTool {
            schema: ToolSchema::new("echo_fs_write", "FS write tool", serde_json::json!({})),
            perms: PermissionSet {
                filesystem: FsPermissions {
                    read_paths: std::collections::HashSet::new(),
                    write_paths: vec![path.to_string()].into_iter().collect(),
                },
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Denied,
            },
        })
    }

    fn make_network_tool(domain: &str) -> Box<dyn Tool> {
        Box::new(NamedTool {
            schema: ToolSchema::new("echo_net", "Network tool", serde_json::json!({})),
            perms: PermissionSet {
                filesystem: FsPermissions::none(),
                network: NetworkPermissions::allow(vec![domain.to_string()]),
                subprocess: SubprocessPolicy::Denied,
            },
        })
    }

    #[tokio::test]
    async fn test_registry_fs_read_permission_denied() {
        let reg = ToolRegistry::new();
        // Tool requires read access to /secret — context grants nothing.
        reg.register(make_fs_read_tool("/secret")).unwrap();
        let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
        let err = reg
            .execute("echo_fs_read", serde_json::json!({}), ctx)
            .await
            .unwrap_err();
        assert!(
            matches!(
                &err,
                RegistryError::PermissionDenied { permission, .. }
                if permission == "filesystem:read"
            ),
            "expected PermissionDenied(filesystem:read), got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_registry_fs_write_permission_denied() {
        let reg = ToolRegistry::new();
        // Tool requires write access to /data — context grants nothing.
        reg.register(make_fs_write_tool("/data")).unwrap();
        let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
        let err = reg
            .execute("echo_fs_write", serde_json::json!({}), ctx)
            .await
            .unwrap_err();
        assert!(
            matches!(
                &err,
                RegistryError::PermissionDenied { permission, .. }
                if permission == "filesystem:write"
            ),
            "expected PermissionDenied(filesystem:write), got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_registry_network_permission_denied() {
        let reg = ToolRegistry::new();
        // Tool requires api.example.com — context grants nothing.
        reg.register(make_network_tool("api.example.com")).unwrap();
        let ctx = ToolContext::new("agent-1", PermissionSet::minimal());
        let err = reg
            .execute("echo_net", serde_json::json!({}), ctx)
            .await
            .unwrap_err();
        assert!(
            matches!(
                &err,
                RegistryError::PermissionDenied { permission, .. }
                if permission == "network"
            ),
            "expected PermissionDenied(network), got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_registry_fs_permission_granted() {
        let reg = ToolRegistry::new();
        // Tool requires read access to /data/logs — context grants exactly that.
        reg.register(make_fs_read_tool("/data/logs")).unwrap();
        let ctx = ToolContext::new(
            "agent-1",
            PermissionSet {
                filesystem: FsPermissions::read_only(vec!["/data/logs".to_string()]),
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Denied,
            },
        );
        let result = reg
            .execute("echo_fs_read", serde_json::json!({}), ctx)
            .await
            .expect("should succeed when permissions are granted");
        assert!(result.success);
    }

    // ── GAP-F4-03: glob path matching ─────────────────────────────────────────

    #[test]
    fn test_fs_path_covered_exact_match() {
        let mut patterns = std::collections::HashSet::new();
        patterns.insert("/tmp/output.txt".to_string());
        assert!(super::fs_path_covered("/tmp/output.txt", &patterns));
        assert!(!super::fs_path_covered("/tmp/other.txt", &patterns));
    }

    #[test]
    fn test_fs_path_covered_glob_wildcard() {
        let mut patterns = std::collections::HashSet::new();
        patterns.insert("/tmp/**".to_string());
        assert!(super::fs_path_covered("/tmp/foo/bar.txt", &patterns));
        assert!(super::fs_path_covered("/tmp/baz", &patterns));
        assert!(!super::fs_path_covered("/var/log/app.log", &patterns));
    }

    #[test]
    fn test_fs_path_covered_glob_single_star() {
        let mut patterns = std::collections::HashSet::new();
        patterns.insert("/data/*.csv".to_string());
        assert!(super::fs_path_covered("/data/sales.csv", &patterns));
        assert!(!super::fs_path_covered("/data/nested/sales.csv", &patterns));
    }

    #[test]
    fn test_fs_path_covered_invalid_pattern_skipped() {
        let mut patterns = std::collections::HashSet::new();
        patterns.insert("[invalid".to_string()); // malformed glob
        patterns.insert("/tmp/**".to_string());
        // Should still match via the valid pattern
        assert!(super::fs_path_covered("/tmp/file", &patterns));
    }

    #[tokio::test]
    async fn test_registry_fs_write_glob_granted() {
        let reg = ToolRegistry::new();
        // Tool declares it needs /tmp/output.txt; context grants /tmp/** via glob.
        reg.register(make_fs_write_tool("/tmp/output.txt")).unwrap();
        let ctx = ToolContext::new(
            "agent-1",
            PermissionSet {
                filesystem: FsPermissions {
                    read_paths: std::collections::HashSet::new(),
                    write_paths: vec!["/tmp/**".to_string()].into_iter().collect(),
                },
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Denied,
            },
        );
        let result = reg
            .execute("echo_fs_write", serde_json::json!({}), ctx)
            .await
            .expect("glob should grant write permission");
        assert!(result.success);
    }

    #[tokio::test]
    async fn test_registry_fs_write_glob_denied_outside_pattern() {
        let reg = ToolRegistry::new();
        // Tool declares /var/log/app.log; context only grants /tmp/**
        reg.register(make_fs_write_tool("/var/log/app.log")).unwrap();
        let ctx = ToolContext::new(
            "agent-1",
            PermissionSet {
                filesystem: FsPermissions {
                    read_paths: std::collections::HashSet::new(),
                    write_paths: vec!["/tmp/**".to_string()].into_iter().collect(),
                },
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Denied,
            },
        );
        let err = reg
            .execute("echo_fs_write", serde_json::json!({}), ctx)
            .await
            .unwrap_err();
        assert!(
            matches!(
                &err,
                RegistryError::PermissionDenied { permission, .. }
                if permission == "filesystem:write"
            ),
            "expected PermissionDenied(filesystem:write), got {err:?}"
        );
    }

    // ── 4B: 审计日志上限测试 ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_registry_audit_log_max_entries() {
        // max_audit_entries = 10 → after 20 calls the log should have been evicted
        // at least once; total entries must not exceed max + to_remove (11).
        let reg = ToolRegistry::new().with_max_audit_entries(10);
        reg.register(make_echo_tool()).unwrap();
        let ctx = default_ctx();

        for i in 0..20u64 {
            reg.execute("echo", serde_json::json!({"i": i}), ctx.clone())
                .await
                .unwrap();
        }

        // After eviction(s) the log length must be ≤ 11 (max + 10 % slack).
        let log_len = reg.recent_log(100).await.len();
        assert!(
            log_len <= 11,
            "expected audit log ≤ 11 entries after eviction, got {log_len}"
        );
    }

    // ─── JoinSet 超时测试 (Agent 4: Red Phase) ────────────────────────────────

    /// Test: 工具超时取消不会引发 Panic
    ///
    /// 验证使用 JoinSet 实现后，超时取消工具执行是安全的，不会 panic
    #[tokio::test]
    async fn test_registry_execute_timeout_joinset() {
        let reg = ToolRegistry::new();
        reg.register(make_slow_tool()).unwrap();

        // 执行会超时的工具
        let result = reg
            .execute("slow", serde_json::json!({}), default_ctx())
            .await;

        // 验证：应该返回 Ok(ToolResult) 而不是 Err
        assert!(result.is_ok(), "超时应该返回 Ok(ToolResult)，不应返回 Err");

        let tool_result = result.unwrap();
        // 验证：ToolResult 应该标记为失败且包含超时错误
        assert!(!tool_result.success, "超时工具应返回 success=false");
        let err = tool_result.error.as_ref().expect("应该有错误信息");
        assert_eq!(
            err.code,
            crate::types::ToolErrorCode::Timeout,
            "错误类型应该是 Timeout"
        );

        // 关键验证：此测试不应 panic，证明 JoinSet 的超时取消是安全的
    }

    /// Test: 并发执行多个工具验证 JoinSet 能正确管理多个任务
    ///
    /// 这个测试验证 JoinSet 能够正确管理并发执行的多个工具任务
    #[tokio::test]
    async fn test_registry_concurrent_tools_execution() {
        use std::sync::Arc;

        let reg = Arc::new(ToolRegistry::new());

        // 注册多个工具
        reg.register(make_echo_tool()).unwrap();

        // 创建另一个快速工具
        struct QuickTool;
        #[async_trait]
        impl Tool for QuickTool {
            fn name(&self) -> &str {
                "quick"
            }
            fn description(&self) -> &str {
                "Quick tool"
            }
            fn schema(&self) -> &ToolSchema {
                static SCHEMA: std::sync::OnceLock<ToolSchema> = std::sync::OnceLock::new();
                SCHEMA.get_or_init(|| ToolSchema::new("quick", "Quick tool", serde_json::json!({})))
            }
            fn permissions(&self) -> &PermissionSet {
                static PERMS: std::sync::OnceLock<PermissionSet> = std::sync::OnceLock::new();
                PERMS.get_or_init(PermissionSet::minimal)
            }
            async fn execute(&self, args: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
                // 短暂延迟模拟实际工作
                tokio::time::sleep(Duration::from_millis(10)).await;
                ToolResult::ok(args, 0)
            }
        }

        reg.register(Box::new(QuickTool)).unwrap();

        let ctx = default_ctx();

        // 并发执行多个工具调用
        let mut handles = vec![];
        for i in 0..5u64 {
            let reg_clone = Arc::clone(&reg);
            let ctx_clone = ctx.clone();
            handles.push(tokio::spawn(async move {
                reg_clone
                    .execute("echo", serde_json::json!({"i": i}), ctx_clone)
                    .await
            }));
        }

        // 等待所有任务完成
        let mut success_count = 0;
        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result.is_ok(), "并发执行不应返回错误");
            if result.unwrap().success {
                success_count += 1;
            }
        }

        assert_eq!(success_count, 5, "所有并发工具调用都应该成功");

        // 验证审计日志记录了所有调用
        let log = reg.recent_log(10).await;
        assert_eq!(log.len(), 5, "审计日志应该记录5条记录");
    }

    // ── GAP-F4-02: Global execution mode tests ────────────────────────────────

    /// 新建 ToolRegistry 默认应处于 Safe 模式
    #[test]
    fn test_registry_default_mode_is_safe() {
        let reg = ToolRegistry::new();
        assert_eq!(reg.global_mode(), RegistryExecutionMode::Safe);
    }

    /// 正确 power_key 可以进入 Power 模式
    #[test]
    fn test_enter_power_mode_with_valid_key() {
        struct AlwaysOkHash;
        impl PowerKeyVerify for AlwaysOkHash {
            fn verify(&self, _candidate: &str) -> bool { true }
        }

        let reg = ToolRegistry::new();
        assert_eq!(reg.global_mode(), RegistryExecutionMode::Safe);
        let _guard = reg.enter_power_mode("any-key", &AlwaysOkHash).expect("valid key should succeed");
        assert_eq!(reg.global_mode(), RegistryExecutionMode::Power);
    }

    /// 错误 power_key 被拒绝，模式保持 Safe
    #[test]
    fn test_enter_power_mode_with_invalid_key_stays_safe() {
        struct AlwaysFailHash;
        impl PowerKeyVerify for AlwaysFailHash {
            fn verify(&self, _candidate: &str) -> bool { false }
        }

        let reg = ToolRegistry::new();
        let result = reg.enter_power_mode("wrong-key", &AlwaysFailHash);
        assert!(result.is_err(), "invalid key should be rejected");
        assert_eq!(reg.global_mode(), RegistryExecutionMode::Safe, "mode must remain Safe");
    }

    /// dropping the guard reverts the registry to Safe Mode (regression: GAP-F8 fix)
    #[test]
    fn test_power_mode_guard_drop_resets_to_safe() {
        struct AlwaysOkHash;
        impl PowerKeyVerify for AlwaysOkHash {
            fn verify(&self, _candidate: &str) -> bool { true }
        }

        let reg = ToolRegistry::new();
        {
            let _guard = reg.enter_power_mode("any-key", &AlwaysOkHash)
                .expect("valid key should succeed");
            assert_eq!(reg.global_mode(), RegistryExecutionMode::Power,
                "registry must be in Power mode while guard is alive");
        } // _guard dropped here

        assert_eq!(reg.global_mode(), RegistryExecutionMode::Safe,
            "registry must revert to Safe mode after guard is dropped");
    }

    /// 全局 Safe 模式时，ctx.execution_mode = Power 的调用者不能绕过权限检查
    #[tokio::test]
    async fn test_global_safe_blocks_ctx_power_override() {
        use crate::types::{FsPermissions, SubprocessPolicy};

        // Tool requires subprocess
        struct SubprocTool {
            schema: ToolSchema,
            perms: PermissionSet,
        }

        #[async_trait]
        impl Tool for SubprocTool {
            fn name(&self) -> &str { "subproc" }
            fn description(&self) -> &str { "needs subprocess" }
            fn schema(&self) -> &ToolSchema { &self.schema }
            fn permissions(&self) -> &PermissionSet { &self.perms }
            async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
                ToolResult::ok(serde_json::json!("ok"), 0)
            }
        }

        let reg = ToolRegistry::new();
        reg.register(Box::new(SubprocTool {
            schema: ToolSchema::new("subproc", "subprocess test", serde_json::json!({})),
            perms: PermissionSet {
                filesystem: FsPermissions::none(),
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Allowed,
            },
        })).unwrap();

        // ctx says Power, but global registry is Safe
        let ctx = ToolContext::with_mode(
            "attacker",
            PermissionSet::minimal(), // minimal = subprocess Denied
            RegistryExecutionMode::Power,
        );

        let result = reg.execute("subproc", serde_json::json!({}), ctx).await;
        assert!(
            matches!(result, Err(RegistryError::PermissionDenied { .. })),
            "Global Safe must block ctx Power override; got {:?}", result
        );
    }

    /// 全局 Power 模式时，subprocess 权限检查被跳过
    #[tokio::test]
    async fn test_global_power_mode_bypasses_permission_checks() {
        use crate::types::SubprocessPolicy;

        struct SubprocTool2 {
            schema: ToolSchema,
            perms: PermissionSet,
        }

        #[async_trait]
        impl Tool for SubprocTool2 {
            fn name(&self) -> &str { "subproc2" }
            fn description(&self) -> &str { "subprocess test" }
            fn schema(&self) -> &ToolSchema { &self.schema }
            fn permissions(&self) -> &PermissionSet { &self.perms }
            async fn execute(&self, _: serde_json::Value, _: &ToolContext) -> ToolResult {
                ToolResult::ok(serde_json::json!("ok"), 0)
            }
        }

        struct AlwaysOk;
        impl PowerKeyVerify for AlwaysOk {
            fn verify(&self, _: &str) -> bool { true }
        }

        let reg = ToolRegistry::new();
        reg.register(Box::new(SubprocTool2 {
            schema: ToolSchema::new("subproc2", "subprocess test", serde_json::json!({})),
            perms: PermissionSet {
                filesystem: FsPermissions::none(),
                network: NetworkPermissions::none(),
                subprocess: SubprocessPolicy::Allowed,
            },
        })).unwrap();

        // Switch registry to Power mode — keep the guard alive for the duration of this test.
        let _guard = reg.enter_power_mode("valid-key", &AlwaysOk).unwrap();

        // ctx with minimal (subprocess Denied) — should succeed because global=Power
        let ctx = ToolContext::new("agent", PermissionSet::minimal());
        let result = reg.execute("subproc2", serde_json::json!({}), ctx).await;
        assert!(result.is_ok(), "Power mode should bypass checks; got {:?}", result);
        assert!(result.unwrap().success);
    }

    // ── G-1 audit writer integration tests ────────────────────────────────────

    #[tokio::test]
    async fn test_audit_writer_tool_call_and_result_written() {
        use crate::audit::{AuditEvent, AuditLogConfig, AuditLogWriter};

        let config = AuditLogConfig::new().with_max_memory_entries(50);
        let (handle, store, _task) = AuditLogWriter::start(config);

        let reg = ToolRegistry::new().with_audit_writer(handle);
        reg.register(make_echo_tool()).unwrap();

        let ctx = default_ctx();
        let args = serde_json::json!({"msg": "hello"});
        let result = reg.execute("echo", args.clone(), ctx).await.unwrap();
        assert!(result.success);

        // Give the background writer a tick to process events.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let entries = store.list(50, Some("agent-1"), None);
        let call_entry = entries.iter().find(|e| e.event_type() == "TOOL_CALL");
        let result_entry = entries.iter().find(|e| e.event_type() == "TOOL_RESULT");

        assert!(call_entry.is_some(), "ToolCall event must be in AuditStore");
        assert!(result_entry.is_some(), "ToolResult event must be in AuditStore");

        if let AuditEvent::ToolCall { tool_name, agent_id, args: logged_args, .. } =
            call_entry.unwrap()
        {
            assert_eq!(tool_name, "echo");
            assert_eq!(agent_id, "agent-1");
            assert_eq!(logged_args.as_ref().unwrap()["msg"], "hello");
        } else {
            panic!("expected ToolCall variant");
        }

        if let AuditEvent::ToolResult { tool_name, success, error_code, .. } =
            result_entry.unwrap()
        {
            assert_eq!(tool_name, "echo");
            assert!(*success);
            assert!(error_code.is_none());
        } else {
            panic!("expected ToolResult variant");
        }
    }

    #[tokio::test]
    async fn test_audit_writer_enter_power_mode_written() {
        use crate::audit::{AuditEvent, AuditLogConfig, AuditLogWriter};

        let config = AuditLogConfig::new().with_max_memory_entries(50);
        let (handle, store, _task) = AuditLogWriter::start(config);

        struct AlwaysOkHash;
        impl PowerKeyVerify for AlwaysOkHash {
            fn verify(&self, _: &str) -> bool {
                true
            }
        }

        let reg = ToolRegistry::new().with_audit_writer(handle);
        let _guard = reg.enter_power_mode("any-key", &AlwaysOkHash).unwrap();

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // ModeSwitch uses "system" as agent_id — query without agent filter
        let entries = store.list(50, None, None);
        let mode_entry = entries.iter().find(|e| e.event_type() == "MODE_SWITCH");
        assert!(
            mode_entry.is_some(),
            "ModeSwitch event must be in AuditStore after enter_power_mode"
        );

        if let AuditEvent::ModeSwitch { from_mode, to_mode, reason, agent_id, .. } =
            mode_entry.unwrap()
        {
            assert_eq!(from_mode, "safe");
            assert_eq!(to_mode, "power");
            assert_eq!(reason, "power_key_verified");
            assert_eq!(agent_id, "system");
        } else {
            panic!("expected ModeSwitch variant");
        }
    }

    #[tokio::test]
    async fn test_audit_writer_timeout_writes_error_code() {
        use crate::audit::{AuditEvent, AuditLogConfig, AuditLogWriter};

        let config = AuditLogConfig::new().with_max_memory_entries(50);
        let (handle, store, _task) = AuditLogWriter::start(config);

        let reg = ToolRegistry::new().with_audit_writer(handle);
        reg.register(make_slow_tool()).unwrap();

        let ctx = default_ctx();
        let result = reg.execute("slow", serde_json::json!({}), ctx).await.unwrap();
        assert!(!result.success);

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let entries = store.list(50, Some("agent-1"), None);
        let result_entry = entries.iter().find(|e| e.event_type() == "TOOL_RESULT");
        assert!(result_entry.is_some(), "ToolResult event must be written on timeout");

        if let AuditEvent::ToolResult { success, error_code, .. } = result_entry.unwrap() {
            assert!(!*success);
            assert_eq!(error_code.as_deref(), Some("timeout"));
        } else {
            panic!("expected ToolResult variant");
        }
    }

    #[tokio::test]
    async fn test_no_audit_writer_no_panic() {
        // Registry without audit_writer must not panic on any code path.
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        let ctx = default_ctx();
        let result = reg.execute("echo", serde_json::json!({}), ctx).await;
        assert!(result.is_ok());
    }

    // ── G-2: SandboxApplier integration tests ────────────────────────────────

    use std::sync::atomic::AtomicU32;

    /// Applier that succeeds and counts calls.
    struct TrackingApplier {
        calls: Arc<AtomicU32>,
    }
    impl crate::sandbox::SandboxApplier for TrackingApplier {
        fn apply_safe_mode(&self) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    /// Applier that always fails.
    struct BrokenApplier;
    impl crate::sandbox::SandboxApplier for BrokenApplier {
        fn apply_safe_mode(&self) -> Result<(), String> {
            Err("simulated OS sandbox failure".into())
        }
    }

    /// `apply_safe_mode()` is called exactly once even with concurrent execute() calls.
    #[tokio::test]
    async fn test_sandbox_applier_called_at_most_once() {
        let calls = Arc::new(AtomicU32::new(0));
        let applier = Arc::new(TrackingApplier { calls: Arc::clone(&calls) });

        let reg = ToolRegistry::new().with_sandbox_applier(applier);
        reg.register(make_echo_tool()).unwrap();

        // Three sequential Safe-mode executions — applier must be called only once.
        for _ in 0..3 {
            let ctx = default_ctx();
            let r = reg.execute("echo", serde_json::json!({}), ctx).await;
            assert!(r.is_ok(), "execute should succeed after sandbox is applied");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1, "apply_safe_mode must be called exactly once");
    }

    /// When no sandbox applier is set, execute() succeeds as before (backward-compat).
    #[tokio::test]
    async fn test_no_sandbox_applier_still_works() {
        let reg = ToolRegistry::new();
        reg.register(make_echo_tool()).unwrap();
        let ctx = default_ctx();
        assert!(reg.execute("echo", serde_json::json!({}), ctx).await.is_ok());
    }

    /// When the applier fails, execute() returns ExecutionFailed (fail-closed).
    #[tokio::test]
    async fn test_failing_sandbox_applier_refuses_all_executions() {
        let reg = ToolRegistry::new().with_sandbox_applier(Arc::new(BrokenApplier));
        reg.register(make_echo_tool()).unwrap();

        for _ in 0..2 {
            let ctx = default_ctx();
            let err = reg.execute("echo", serde_json::json!({}), ctx).await;
            assert!(
                matches!(err, Err(RegistryError::ExecutionFailed(_))),
                "broken applier must produce ExecutionFailed, got: {:?}", err
            );
        }
    }

    /// In Power mode the sandbox applier is NOT invoked.
    #[tokio::test]
    async fn test_sandbox_applier_skipped_in_power_mode() {
        let calls = Arc::new(AtomicU32::new(0));
        let applier = Arc::new(TrackingApplier { calls: Arc::clone(&calls) });

        let reg = ToolRegistry::new().with_sandbox_applier(applier);
        reg.register(make_echo_tool()).unwrap();

        // Enter Power mode with a trivially-verifying key.
        struct TrueHash;
        impl PowerKeyVerify for TrueHash { fn verify(&self, _: &str) -> bool { true } }
        let _guard = reg.enter_power_mode("any", &TrueHash).expect("power mode");

        let ctx = ToolContext::with_mode("agent", PermissionSet::minimal(), RegistryExecutionMode::Power);
        reg.execute("echo", serde_json::json!({}), ctx).await.unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 0, "applier must NOT be called in Power mode");
    }

    /// `NoopSandboxApplier` integrates cleanly and succeeds.
    #[tokio::test]
    async fn test_noop_sandbox_applier_integration() {
        let reg = ToolRegistry::new()
            .with_sandbox_applier(crate::sandbox::NoopSandboxApplier::new());
        reg.register(make_echo_tool()).unwrap();
        let ctx = default_ctx();
        assert!(reg.execute("echo", serde_json::json!({}), ctx).await.is_ok());
    }
}
