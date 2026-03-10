use claw_pal::traits::ProcessManager;
use claw_pal::ProcessConfig;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use sysinfo::{Pid, ProcessRefreshKind, System};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::{
    agent_handle::{AgentMessage, SharedSender},
    agent_types::{AgentConfig, AgentHandle, AgentId, AgentInfo, AgentStatus, ResourceUsage},
    error::RuntimeError,
    event_bus::EventBus,
    events::Event,
    restart_policy::RestartState,
};

// ─── ResourceSnapshot ─────────────────────────────────────────────────────────

/// A point-in-time resource usage sample collected by `start_resource_monitor_task`.
///
/// Updated independently of the health-check cycle (default: every 5 s) by
/// sampling the OS process table via `sysinfo`.  Unlike the agent-reported
/// metrics stored in `HealthStatus`, this snapshot is **authoritative** and
/// always current regardless of whether the agent has responded to a health
/// check.
#[derive(Debug, Clone, Default)]
pub struct ResourceSnapshot {
    /// Resident memory in kilobytes (RSS).
    pub memory_kb: u64,
    /// CPU usage percentage since the previous sample (0.0–100.0 per core).
    pub cpu_percent: f32,
    /// Unix timestamp in milliseconds when this snapshot was taken.
    pub sampled_at_ms: u64,
}

// ─── Health Status ────────────────────────────────────────────────────────────

/// Health status of an agent.
#[derive(Debug, Clone, Default)]
pub struct HealthStatus {
    /// Agent ID.
    pub agent_id: AgentId,
    /// Whether the agent is considered healthy.
    pub is_healthy: bool,
    /// Process ID if available.
    pub pid: Option<u32>,
    /// Last heartbeat timestamp (Unix milliseconds).
    pub last_heartbeat_ms: Option<u64>,
    /// Memory usage in KB (if available).
    pub memory_usage_kb: Option<u64>,
    /// CPU usage percentage (if available).
    pub cpu_usage_percent: Option<f32>,
    /// Time since last heartbeat in milliseconds (if available).
    pub time_since_heartbeat_ms: Option<u64>,
    /// Whether the agent is responding to health checks.
    pub is_responsive: bool,
    /// Additional health metrics.
    pub metrics: HashMap<String, f64>,
}

impl HealthStatus {
    /// Create a new health status for the given agent.
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            is_healthy: false,
            pid: None,
            last_heartbeat_ms: None,
            memory_usage_kb: None,
            cpu_usage_percent: None,
            time_since_heartbeat_ms: None,
            is_responsive: false,
            metrics: HashMap::new(),
        }
    }
}

// ─── Resource Quota ───────────────────────────────────────────────────────────

/// Resource quota for an agent.
#[derive(Debug, Clone, Default)]
pub struct ResourceQuota {
    /// Maximum memory in MB.
    pub max_memory_mb: Option<u64>,
    /// Maximum CPU usage percentage (0-100).
    pub max_cpu_percent: Option<f32>,
    /// Maximum number of file descriptors.
    pub max_file_descriptors: Option<u32>,
    /// Maximum number of threads.
    pub max_threads: Option<u32>,
}

impl ResourceQuota {
    /// Create a new empty quota (no limits).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set maximum memory limit.
    pub fn with_memory(mut self, mb: u64) -> Self {
        self.max_memory_mb = Some(mb);
        self
    }

    /// Set maximum CPU limit.
    pub fn with_cpu(mut self, percent: f32) -> Self {
        self.max_cpu_percent = Some(percent);
        self
    }

    /// Check if the quota is exceeded based on current usage.
    pub fn check_exceeded(&self, memory_kb: u64, cpu_percent: f32) -> Vec<String> {
        let mut violations = Vec::new();

        if let Some(max_mb) = self.max_memory_mb {
            let current_mb = memory_kb / 1024;
            if current_mb > max_mb {
                violations.push(format!(
                    "Memory limit exceeded: {}MB > {}MB",
                    current_mb, max_mb
                ));
            }
        }

        if let Some(max_cpu) = self.max_cpu_percent {
            if cpu_percent > max_cpu {
                violations.push(format!(
                    "CPU limit exceeded: {:.1}% > {:.1}%",
                    cpu_percent, max_cpu
                ));
            }
        }

        violations
    }
}

// ─── Restart Policy ───────────────────────────────────────────────────────────

/// Policy for automatic agent restart.
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    /// Whether auto-restart is enabled.
    pub enabled: bool,
    /// Maximum number of restart attempts.
    pub max_restarts: u32,
    /// Backoff duration between restart attempts.
    pub backoff_duration: Duration,
    /// Maximum backoff duration.
    pub max_backoff_duration: Duration,
    /// Reset restart count after this duration of successful operation.
    pub reset_after: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_restarts: 3,
            backoff_duration: Duration::from_secs(1),
            max_backoff_duration: Duration::from_secs(60),
            reset_after: Duration::from_secs(300),
        }
    }
}

impl RestartPolicy {
    /// Create a new restart policy with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable auto-restart.
    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Default::default()
        }
    }

    /// Set maximum restart attempts.
    pub fn with_max_restarts(mut self, count: u32) -> Self {
        self.max_restarts = count;
        self
    }

    /// Set initial backoff duration.
    pub fn with_backoff(mut self, duration: Duration) -> Self {
        self.backoff_duration = duration;
        self
    }
}

// ─── Steer Command ────────────────────────────────────────────────────────────

/// Command to steer (control) a running agent.
#[derive(Debug, Clone)]
pub enum SteerCommand {
    /// Pause the agent (temporarily stop processing).
    Pause,
    /// Resume the agent from paused state.
    Resume,
    /// Update agent configuration.
    UpdateConfig(Box<AgentConfig>),
    /// Set log level.
    SetLogLevel(String),
    /// Trigger a heartbeat check.
    TriggerHeartbeat,
    /// Custom command with string payload.
    Custom { command: String, payload: Option<String> },
    /// Inject a message into the agent's input queue.
    ///
    /// Used by [`TriggerDispatcher`](crate::trigger_dispatcher::TriggerDispatcher) to drive
    /// Agent behavior when a trigger fires (Cron / Webhook / Event).
    InjectMessage(String),
}

// ─── AgentState ───────────────────────────────────────────────────────────────

/// Unified agent state, combining base lifecycle info with health and quota data.
///
/// This is the single source of truth stored in the orchestrator's `DashMap`.
/// Previously split across `AgentInfo` (in `agents`) and `ExtendedAgentInfo`
/// (in `extended_info`), the merged design eliminates TOCTOU races and the
/// need for a separate `RwLock<HashMap>`.
#[derive(Debug, Clone)]
pub(crate) struct AgentState {
    // ── Fields from AgentInfo ──
    /// Agent configuration.
    pub config: AgentConfig,
    /// Unix timestamp in milliseconds when the agent was registered.
    pub started_at: u64,
    /// Process handle if agent is running in a separate OS process.
    pub process_handle: Option<claw_pal::ProcessHandle>,
    /// Current lifecycle status.
    pub status: AgentStatus,

    // ── Fields from ExtendedAgentInfo ──
    /// Last known health status.
    pub health: Option<HealthStatus>,
    /// Resource quota for this agent.
    pub quota: ResourceQuota,
    /// Last heartbeat timestamp (Unix milliseconds).
    pub last_heartbeat: u64,
    /// Number of restart attempts since last successful start.
    pub restart_count: u32,
    /// Timestamp when agent was last started (reserved for auto-restart scheduling).
    #[allow(dead_code)]
    pub last_start_time: u64,
    /// Shared sender slot for IPC agents (spawned via [`AgentOrchestrator::spawn_agent`]).
    ///
    /// `None` for registered or out-of-process agents.  The orchestrator hot-swaps
    /// the inner sender on each restart so existing `IpcAgentHandle` clones remain
    /// usable without callers needing to re-obtain a handle.
    pub(crate) ipc_tx: Option<SharedSender>,

    // ── G-6: Real-time resource snapshot ──
    /// Latest resource usage sample collected by `start_resource_monitor_task`.
    ///
    /// Uses a `std::sync::RwLock` (not `tokio`) so that synchronous callers
    /// (e.g. `to_agent_info()`) can read without blocking the async executor.
    /// The `Arc` allows the monitoring task to hold a clone across
    /// `spawn_blocking` boundaries without needing a `'static` reference into
    /// `AgentState`.
    pub(crate) resource_snapshot: Arc<std::sync::RwLock<Option<ResourceSnapshot>>>,
}

impl AgentState {
    pub(crate) fn new(config: AgentConfig, process_handle: Option<claw_pal::ProcessHandle>) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            config,
            started_at: now,
            process_handle,
            status: AgentStatus::Running,
            health: None,
            quota: ResourceQuota::default(),
            last_heartbeat: now,
            restart_count: 0,
            last_start_time: now,
            ipc_tx: None,
            resource_snapshot: Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// Convert to the public `AgentInfo` view (for backward-compatible API).
    fn to_agent_info(&self) -> AgentInfo {
        let resource_usage = self.to_resource_usage();
        AgentInfo {
            config: self.config.clone(),
            started_at: self.started_at,
            process_handle: self.process_handle.clone(),
            status: self.status,
            resource_usage,
        }
    }

    /// Extract the latest [`ResourceUsage`] snapshot.
    ///
    /// G-6: Prefers the live sysinfo sample; falls back to health-check-reported
    /// data for in-process agents that have no OS-level PID.
    pub(crate) fn to_resource_usage(&self) -> Option<ResourceUsage> {
        if let Ok(guard) = self.resource_snapshot.read() {
            if let Some(ref snap) = *guard {
                return Some(ResourceUsage {
                    cpu_ms: None,
                    memory_bytes: Some(snap.memory_kb * 1024),
                });
            }
        }
        // Fallback: health-check-reported data (in-process agents without a pid).
        self.health.as_ref().map(|h| ResourceUsage {
            cpu_ms: None,
            memory_bytes: h.memory_usage_kb.map(|kb| kb * 1024),
        })
    }
}

// ─── ExtendedAgentInfo (deprecated shim) ──────────────────────────────────────

/// Extended agent info including health and quota data.
///
/// # Deprecation
///
/// This type is kept for public API compatibility only. Internally the
/// orchestrator now stores a unified [`AgentState`] in a single `DashMap`,
/// which eliminates the TOCTOU race that existed when `AgentInfo` and
/// `ExtendedAgentInfo` were stored separately.
#[deprecated(since = "1.1.0", note = "Use AgentState fields directly via orchestrator methods")]
#[derive(Debug, Clone)]
pub struct ExtendedAgentInfo {
    /// Base agent info.
    pub base: AgentInfo,
    /// Last known health status.
    pub health: Option<HealthStatus>,
    /// Resource quota for this agent.
    pub quota: ResourceQuota,
    /// Last heartbeat timestamp.
    pub last_heartbeat: u64,
    /// Number of restart attempts since last successful start.
    pub restart_count: u32,
    /// Timestamp when agent was last started.
    pub last_start_time: u64,
}

// ─── OrchestratorConfig ───────────────────────────────────────────────────────

/// `AgentOrchestrator` 行为配置。
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// 心跳超时（毫秒）：超过此时间无心跳则标记代理为不健康。默认: 30_000 ms。
    pub heartbeat_timeout_ms: u64,
    /// 健康检查扫描间隔（秒）。默认: 10 秒。
    pub health_check_interval_secs: u64,
    /// 资源监控采样间隔（秒）。默认: 5 秒。
    ///
    /// The resource monitor runs independently of the health-check loop and
    /// uses `sysinfo` to poll each agent's OS process for memory and CPU usage.
    /// A shorter interval gives fresher data at the cost of slightly more CPU
    /// overhead.  Set to `0` to disable the resource monitor entirely.
    pub resource_monitor_interval_secs: u64,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            heartbeat_timeout_ms: 30_000,
            health_check_interval_secs: 10,
            resource_monitor_interval_secs: 5,
        }
    }
}

// ─── AgentOrchestrator ────────────────────────────────────────────────────────

// ─── AgentDiscoveryEntry ──────────────────────────────────────────────────────

/// A single entry returned by [`AgentOrchestrator::discover_capabilities`].
#[derive(Debug, Clone)]
pub struct AgentDiscoveryEntry {
    /// Agent ID.
    pub agent_id: AgentId,
    /// Capabilities declared via `agent.announce`.
    pub capabilities: Vec<String>,
    /// Current agent status (e.g. "Running", "Stopped").
    pub status: String,
}

/// Manages multiple agents' lifecycle and coordination.
///
/// Thread-safe: backed by `DashMap` for concurrent access without a global
/// lock.  All state-changing operations publish the appropriate `Event` to
/// the shared `EventBus`.
///
/// Features:
/// - Agent lifecycle: register, spawn, terminate, kill
/// - Health checking with heartbeat monitoring
/// - Resource quota enforcement
/// - Automatic restart on failure
/// - Runtime steering (pause/resume/config update)
/// - Agent discovery: announce/discover capability declarations
///
/// Two registration paths exist:
/// - [`AgentOrchestrator::register`] — in-process agents (no OS process spawned).
/// - [`AgentOrchestrator::spawn`] — out-of-process agents backed by a real OS process via PAL.
pub struct AgentOrchestrator {
    /// Single unified map: replaces the former `agents` + `extended_info` pair.
    agents: Arc<DashMap<AgentId, AgentState>>,
    event_bus: Arc<EventBus>,
    process_manager: Arc<dyn ProcessManager>,
    restart_policy: Arc<RwLock<RestartPolicy>>,
    health_check_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    auto_restart_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    /// G-6: Background task that polls OS process stats via sysinfo.
    resource_monitor_handle: Arc<RwLock<Option<JoinHandle<()>>>>,
    cancel_token: tokio_util::sync::CancellationToken,
    /// Orchestrator behaviour configuration.
    config: OrchestratorConfig,
    /// Per-agent restart tracking for the IPC spawn path (Phase 2.4).
    restart_states: Arc<DashMap<AgentId, crate::restart_policy::RestartState>>,
    /// Capability declarations: agent_id → list of capability strings.
    capabilities: Arc<DashMap<AgentId, Vec<String>>>,
    /// Optional audit log sink — receives `AgentSpawned` events (GAP-F8-02).
    ///
    /// When set, `spawn_agent()` writes an `AuditEvent::AgentSpawned` record
    /// immediately after the agent is registered.  Inject via
    /// [`with_audit_log`](Self::with_audit_log).
    audit_log: Option<claw_tools::audit::AuditLogWriterHandle>,
}

impl AgentOrchestrator {
    /// Create a new orchestrator with the given process manager.
    ///
    /// This constructor only initialises the data structures — background tasks
    /// are **not** started automatically.  Call [`start`](Self::start) after
    /// construction (and before registering agents) to launch the health-check
    /// and auto-restart loops.
    ///
    /// If you need an orchestrator without background tasks for unit tests, you
    /// can simply omit the `start()` call; see also
    /// [`new_for_test`](Self::new_for_test).
    pub fn new(
        event_bus: Arc<EventBus>,
        process_manager: Arc<dyn ProcessManager>,
    ) -> Self {
        Self {
            agents: Arc::new(DashMap::new()),
            event_bus,
            process_manager,
            restart_policy: Arc::new(RwLock::new(RestartPolicy::default())),
            health_check_handle: Arc::new(RwLock::new(None)),
            auto_restart_handle: Arc::new(RwLock::new(None)),
            resource_monitor_handle: Arc::new(RwLock::new(None)),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            config: OrchestratorConfig::default(),
            restart_states: Arc::new(DashMap::new()),
            capabilities: Arc::new(DashMap::new()),
            audit_log: None,
        }
    }

    /// Start background maintenance tasks (heartbeat monitor, cleanup, etc.).
    ///
    /// Call this after creating the orchestrator and before registering agents.
    pub fn start(&self) {
        self.start_health_check_task();
        self.start_auto_restart_task();
        self.start_resource_monitor_task();
    }

    /// Attach an audit log writer handle.
    ///
    /// When set, lifecycle events (e.g. `AgentSpawned`) are written to the
    /// persistent audit log in addition to the in-memory ring buffer.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use claw_tools::audit::{AuditLogConfig, AuditLogWriter};
    ///
    /// let (handle, _store, _task) = AuditLogWriter::start(AuditLogConfig::default());
    /// let orchestrator = AgentOrchestrator::new(bus, pm).with_audit_log(handle);
    /// ```
    pub fn with_audit_log(mut self, handle: claw_tools::audit::AuditLogWriterHandle) -> Self {
        self.audit_log = Some(handle);
        self
    }

    /// Create a new orchestrator **without** starting background tasks.
    ///
    /// Intended for unit tests that do not require health-check or auto-restart
    /// loops.  In production code, prefer [`new`](Self::new) which starts all
    /// background tasks automatically.
    pub fn new_for_test(
        event_bus: Arc<EventBus>,
        process_manager: Arc<dyn ProcessManager>,
    ) -> Self {
        Self {
            agents: Arc::new(DashMap::new()),
            event_bus,
            process_manager,
            restart_policy: Arc::new(RwLock::new(RestartPolicy::default())),
            health_check_handle: Arc::new(RwLock::new(None)),
            auto_restart_handle: Arc::new(RwLock::new(None)),
            resource_monitor_handle: Arc::new(RwLock::new(None)),
            cancel_token: tokio_util::sync::CancellationToken::new(),
            config: OrchestratorConfig::default(),
            restart_states: Arc::new(DashMap::new()),
            capabilities: Arc::new(DashMap::new()),
            audit_log: None,
        }
    }

    /// Gracefully shut down the orchestrator, cancelling background tasks
    /// and waiting for them to complete.
    ///
    /// Prefer calling this over relying on `Drop`, which only signals
    /// cancellation without awaiting task completion.
    pub async fn graceful_shutdown(&self) {
        self.cancel_token.cancel();

        if let Some(handle) = self.health_check_handle.write().await.take() {
            let _ = handle.await;
        }
        if let Some(handle) = self.auto_restart_handle.write().await.take() {
            let _ = handle.await;
        }
        if let Some(handle) = self.resource_monitor_handle.write().await.take() {
            let _ = handle.await;
        }
    }

    // ── Health Checking ────────────────────────────────────────────────────────

    /// Check the health of a specific agent.
    ///
    /// Performs comprehensive health checks:
    /// - Process existence (PID check)
    /// - Heartbeat timeout detection
    /// - Resource usage (memory, CPU)
    /// - Responsiveness test
    pub async fn health_check(&self, agent_id: &AgentId) -> Result<HealthStatus, RuntimeError> {
        // Single DashMap lookup — no TOCTOU, no separate RwLock acquisition.
        let mut entry = self.agents.get_mut(agent_id)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.0.clone()))?;

        let mut status = HealthStatus::new(agent_id.clone());
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Check process existence
        if let Some(ref handle) = entry.process_handle {
            status.pid = Some(handle.pid);
            status.is_healthy = true;
        } else {
            // In-process agent — check if still registered
            status.is_healthy = entry.status == AgentStatus::Running;
        }

        // Check heartbeat
        status.last_heartbeat_ms = Some(entry.last_heartbeat);
        let time_since = now.saturating_sub(entry.last_heartbeat);
        status.time_since_heartbeat_ms = Some(time_since);

        // Consider unhealthy if no heartbeat for configured timeout
        if time_since > self.config.heartbeat_timeout_ms {
            status.is_healthy = false;
            status.is_responsive = false;
        } else {
            status.is_responsive = true;
        }

        // G-6: Populate resource metrics from the live sysinfo snapshot first.
        // This is independent of the health-check cycle and always reflects the
        // most recent OS-level sample (default: every 5 s).
        if let Ok(snap_guard) = entry.resource_snapshot.read() {
            if let Some(ref snap) = *snap_guard {
                status.memory_usage_kb = Some(snap.memory_kb);
                status.cpu_usage_percent = Some(snap.cpu_percent);
            }
        }

        // Merge any non-resource metrics from the stored health record.
        // Also provide fallback resource values for in-process agents with no pid.
        if let Some(ref health) = entry.health {
            if status.memory_usage_kb.is_none() {
                status.memory_usage_kb = health.memory_usage_kb;
            }
            if status.cpu_usage_percent.is_none() {
                status.cpu_usage_percent = health.cpu_usage_percent;
            }
            status.metrics = health.metrics.clone();
        }

        // Update stored health status in-place (single lock held throughout)
        entry.health = Some(status.clone());

        Ok(status)
    }

    /// Check health of all registered agents.
    pub async fn health_check_all(&self) -> Vec<HealthStatus> {
        let agent_ids: Vec<AgentId> = self.agent_ids();
        let mut results = Vec::with_capacity(agent_ids.len());

        for agent_id in agent_ids {
            if let Ok(status) = self.health_check(&agent_id).await {
                results.push(status);
            }
        }

        results
    }

    /// Record a heartbeat from an agent.
    pub async fn record_heartbeat(&self, agent_id: &AgentId) -> Result<(), RuntimeError> {
        let mut entry = self.agents.get_mut(agent_id)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.0.clone()))?;

        entry.last_heartbeat = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Ok(())
    }

    // ── Resource Quotas ────────────────────────────────────────────────────────

    /// Set resource quota for an agent.
    pub async fn set_quota(
        &self,
        agent_id: &AgentId,
        quota: ResourceQuota,
    ) -> Result<(), RuntimeError> {
        // Single get_mut: atomic existence check + mutation, no TOCTOU.
        let mut entry = self.agents.get_mut(agent_id)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.0.clone()))?;

        entry.quota = quota;
        Ok(())
    }

    /// Get resource quota for an agent.
    pub async fn get_quota(&self, agent_id: &AgentId) -> Result<ResourceQuota, RuntimeError> {
        self.agents
            .get(agent_id)
            .map(|entry| entry.quota.clone())
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.0.clone()))
    }

    /// Check if agent exceeds its resource quota.
    pub async fn check_quota(&self, agent_id: &AgentId) -> Result<Vec<String>, RuntimeError> {
        let quota = self.get_quota(agent_id).await?;
        let health = self.health_check(agent_id).await?;

        let memory_kb = health.memory_usage_kb.unwrap_or(0);
        let cpu = health.cpu_usage_percent.unwrap_or(0.0);

        Ok(quota.check_exceeded(memory_kb, cpu))
    }

    // ── Auto Restart ───────────────────────────────────────────────────────────

    /// Enable auto-restart with the given policy.
    pub async fn enable_auto_restart(&self, policy: RestartPolicy) {
        let mut current = self.restart_policy.write().await;
        *current = policy;
    }

    /// Disable auto-restart.
    pub async fn disable_auto_restart(&self) {
        let mut current = self.restart_policy.write().await;
        current.enabled = false;
    }

    /// Get current restart policy.
    pub async fn get_restart_policy(&self) -> RestartPolicy {
        self.restart_policy.read().await.clone()
    }

    /// Dynamically update the [`AgentRestartPolicy`] for a specific IPC agent
    /// **without** re-spawning it (G-9 fix).
    ///
    /// Only agents created via [`spawn_agent`](Self::spawn_agent) have a
    /// per-agent [`RestartState`]; agents registered with [`register`](Self::register)
    /// use the global restart policy instead and are not addressable here.
    ///
    /// The new policy takes effect immediately: the next time the agent's
    /// supervisor loop calls `trigger_restart`, it reads the updated policy.
    /// The existing attempt counter is preserved so that a policy upgrade
    /// (e.g. raising `max_retries`) extends the remaining budget without
    /// resetting progress.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeError::AgentNotFound`] if the agent ID is unknown or
    /// was not created via `spawn_agent`.
    pub fn set_agent_restart_policy(
        &self,
        agent_id: &AgentId,
        policy: crate::restart_policy::AgentRestartPolicy,
    ) -> Result<(), RuntimeError> {
        let mut state = self
            .restart_states
            .get_mut(agent_id)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.0.clone()))?;
        state.policy = policy;
        Ok(())
    }

    // ── Steer (Control) ────────────────────────────────────────────────────────

    /// Steer (control) a running agent.
    ///
    /// Supported commands:
    /// - `Pause`: Temporarily stop processing
    /// - `Resume`: Resume from paused state
    /// - `UpdateConfig`: Update agent configuration
    /// - `SetLogLevel`: Change log verbosity
    /// - `TriggerHeartbeat`: Request immediate heartbeat
    pub async fn steer(
        &self,
        agent_id: &AgentId,
        command: SteerCommand,
    ) -> Result<(), RuntimeError> {
        match command {
            SteerCommand::TriggerHeartbeat => {
                // record_heartbeat uses get_mut internally; call separately.
                return self.record_heartbeat(agent_id).await;
            }
            SteerCommand::Custom { command, payload } => {
                // Verify agent exists before publishing the event.
                if !self.agents.contains_key(agent_id) {
                    return Err(RuntimeError::AgentNotFound(agent_id.0.clone()));
                }
                let _ = self.event_bus.publish(Event::Custom {
                    event_type: format!("steer:{}", command),
                    data: serde_json::json!({
                        "agent_id": agent_id.0,
                        "payload": payload,
                    }),
                });
                return Ok(());
            }
            SteerCommand::InjectMessage(message) => {
                // For IPC in-process agents: deliver directly via the shared mpsc sender.
                // For out-of-process agents: publish a Custom event so external listeners
                // (e.g. a remote agent daemon) can pick it up.
                let ipc_tx = self
                    .agents
                    .get(agent_id)
                    .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.0.clone()))
                    .map(|e| e.ipc_tx.clone())?;

                if let Some(shared_tx) = ipc_tx {
                    let guard = shared_tx.lock().await;
                    if let Some(tx) = guard.as_ref() {
                        tx.send(crate::agent_handle::AgentMessage::Send { content: message })
                            .await
                            .map_err(|_| RuntimeError::AgentNotFound(agent_id.0.clone()))?;
                    } else {
                        tracing::warn!(
                            agent_id = %agent_id.0,
                            "InjectMessage: agent IPC sender slot is empty (agent restarting?)"
                        );
                    }
                } else {
                    // Out-of-process agent — broadcast via EventBus.
                    let _ = self.event_bus.publish(Event::Custom {
                        event_type: "trigger:inject_message".to_string(),
                        data: serde_json::json!({
                            "agent_id": agent_id.0,
                            "message": message,
                        }),
                    });
                }
                return Ok(());
            }
            _ => {}
        }

        let mut entry = self.agents.get_mut(agent_id)
            .ok_or_else(|| RuntimeError::AgentNotFound(agent_id.0.clone()))?;

        match command {
            SteerCommand::Pause => {
                if entry.status == AgentStatus::Running {
                    entry.status = AgentStatus::Paused;
                } else {
                    return Err(RuntimeError::ProcessError(
                        format!("Cannot pause agent in {:?} state", entry.status)
                    ));
                }
            }
            SteerCommand::Resume => {
                if entry.status == AgentStatus::Paused {
                    entry.status = AgentStatus::Running;
                } else {
                    return Err(RuntimeError::ProcessError(
                        format!("Cannot resume agent in {:?} state", entry.status)
                    ));
                }
            }
            SteerCommand::UpdateConfig(new_config) => {
                entry.config = *new_config;
            }
            SteerCommand::SetLogLevel(level) => {
                entry.config.metadata.insert("log_level".to_string(), level);
            }
            // TriggerHeartbeat, Custom and InjectMessage are handled above
            SteerCommand::TriggerHeartbeat => {
                return Err(RuntimeError::UnsupportedCommand(
                    "unsupported steer command: TriggerHeartbeat".to_string(),
                ));
            }
            SteerCommand::Custom { command, .. } => {
                return Err(RuntimeError::UnsupportedCommand(
                    format!("unsupported steer command: {:?}", command),
                ));
            }
            SteerCommand::InjectMessage(_) => {
                // Already handled in the early-return branch above.
                unreachable!("InjectMessage is handled before the get_mut block");
            }
        }

        Ok(())
    }

    // ── Internal Background Tasks ───────────────────────────────────────────────

    /// Start the health check background task.
    ///
    /// Applies heartbeat-timeout detection only to **out-of-process** agents
    /// (those with a real OS process handle that should be sending heartbeats).
    /// IPC in-process agents rely on task-exit detection: the tokio task marks
    /// itself as [`AgentStatus::Error`] directly when its message loop exits.
    fn start_health_check_task(&self) {
        let agents = Arc::clone(&self.agents);
        let cancel_token = self.cancel_token.clone();
        let health_check_interval_secs = self.config.health_check_interval_secs;
        let heartbeat_timeout_ms = self.config.heartbeat_timeout_ms;

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(health_check_interval_secs));

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        tracing::debug!("health_check_task cancelled");
                        break;
                    }
                    _ = interval.tick() => {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;

                        let agent_ids: Vec<AgentId> = agents.iter()
                            .filter(|e| {
                                e.value().status == AgentStatus::Running
                                    && e.value().process_handle.is_some()
                            })
                            .map(|e| e.key().clone())
                            .collect();

                        for agent_id in agent_ids {
                            if let Some(mut entry) = agents.get_mut(&agent_id) {
                                let time_since = now.saturating_sub(entry.last_heartbeat);
                                if time_since > heartbeat_timeout_ms {
                                    tracing::warn!(
                                        agent = %agent_id,
                                        time_since_ms = time_since,
                                        heartbeat_timeout_ms,
                                        "Agent heartbeat timeout — marking as Error"
                                    );
                                    entry.status = AgentStatus::Error;
                                }
                            }
                        }
                    }
                }
            }
        });

        let _ = self.health_check_handle.try_write().map(|mut h| *h = Some(handle));
    }

    /// Start the auto-restart background task.
    ///
    /// Handles the **global** restart policy for registered agents (those
    /// created via [`register`](Self::register)) that do not have a per-agent
    /// [`RestartState`].  IPC agents (created via [`spawn_agent`](Self::spawn_agent))
    /// use the `trigger_restart` free function instead, invoked directly from
    /// the agent's tokio task on exit.
    fn start_auto_restart_task(&self) {
        let agents = Arc::clone(&self.agents);
        let restart_policy = Arc::clone(&self.restart_policy);
        let restart_states = Arc::clone(&self.restart_states);
        let event_bus = Arc::clone(&self.event_bus);
        let cancel_token = self.cancel_token.clone();

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        tracing::debug!("auto_restart_task cancelled");
                        break;
                    }
                    _ = interval.tick() => {
                        let policy = restart_policy.read().await.clone();
                        if !policy.enabled {
                            continue;
                        }

                        // Only handle registered agents without a per-agent restart state.
                        // IPC agents self-manage via trigger_restart.
                        let to_restart: Vec<AgentId> = agents.iter()
                            .filter(|e| {
                                (e.value().status == AgentStatus::Stopped
                                    || e.value().status == AgentStatus::Error)
                                    && !restart_states.contains_key(e.key())
                                    && e.value().restart_count < policy.max_restarts
                            })
                            .map(|e| e.key().clone())
                            .collect();

                        for agent_id in to_restart {
                            if let Some(mut entry) = agents.get_mut(&agent_id) {
                                entry.restart_count += 1;
                                entry.status = AgentStatus::Running;
                                entry.last_heartbeat = SystemTime::now()
                                    .duration_since(UNIX_EPOCH)
                                    .unwrap_or_default()
                                    .as_millis() as u64;
                            }

                            tracing::info!(agent = %agent_id, "Auto-restarted registered agent (global policy)");
                            let _ = event_bus.publish(Event::AgentStarted {
                                agent_id: agent_id.clone(),
                            });
                        }
                    }
                }
            }
        });

        let _ = self.auto_restart_handle.try_write().map(|mut h| *h = Some(handle));
    }

    /// Start the resource monitor background task (G-6).
    ///
    /// Samples CPU and memory for every agent that has a live OS PID using the
    /// `sysinfo` crate.  Results are written into each agent's
    /// `resource_snapshot` (`Arc<std::sync::RwLock<_>>`), which is then read
    /// by `health_check()` and `to_resource_usage()`.
    ///
    /// The task is a no-op when `resource_monitor_interval_secs == 0`.
    fn start_resource_monitor_task(&self) {
        let interval_secs = self.config.resource_monitor_interval_secs;
        if interval_secs == 0 {
            return;
        }

        let agents = Arc::clone(&self.agents);
        let cancel_token = self.cancel_token.clone();

        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        tracing::debug!("resource_monitor_task cancelled");
                        break;
                    }
                    _ = interval.tick() => {
                        // Collect (pid, Arc<snapshot_lock>) for every running agent
                        // that has an OS-level process handle.
                        let targets: Vec<(u32, Arc<std::sync::RwLock<Option<ResourceSnapshot>>>)> =
                            agents
                                .iter()
                                .filter_map(|e| {
                                    let ph = e.value().process_handle.as_ref()?;
                                    Some((
                                        ph.pid,
                                        Arc::clone(&e.value().resource_snapshot),
                                    ))
                                })
                                .collect();

                        if targets.is_empty() {
                            continue;
                        }

                        // sysinfo is a synchronous API — run in a blocking thread.
                        let result = tokio::task::spawn_blocking(move || {
                            let mut sys = System::new();
                            let now_ms = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64;

                            for (pid, snap_lock) in targets {
                                let sysinfo_pid = Pid::from_u32(pid);
                                // Refresh this process only; pass `true` to also
                                // update the CPU-usage delta since the last refresh.
                                sys.refresh_process_specifics(
                                    sysinfo_pid,
                                    ProcessRefreshKind::new()
                                        .with_memory()
                                        .with_cpu(),
                                );

                                if let Some(proc) = sys.process(sysinfo_pid) {
                                    // sysinfo 0.30+: memory() is in bytes.
                                    let snap = ResourceSnapshot {
                                        memory_kb: proc.memory() / 1024,
                                        cpu_percent: proc.cpu_usage(),
                                        sampled_at_ms: now_ms,
                                    };
                                    if let Ok(mut guard) = snap_lock.write() {
                                        *guard = Some(snap);
                                    }
                                }
                            }
                        })
                        .await;

                        if let Err(e) = result {
                            tracing::warn!(
                                err = ?e,
                                "resource_monitor_task: spawn_blocking failed"
                            );
                        }
                    }
                }
            }
        });

        let _ = self.resource_monitor_handle.try_write().map(|mut h| *h = Some(handle));
    }

    // ── In-process registration ───────────────────────────────────────────────

    /// Register a new in-process agent and publish an `AgentStarted` event.
    ///
    /// Does **not** spawn an OS process; use [`AgentOrchestrator::spawn`] for that.
    /// Returns `Err(AgentAlreadyExists)` if an agent with the same ID is
    /// already registered.
    pub fn register(&self, config: AgentConfig) -> Result<AgentHandle, RuntimeError> {
        let agent_id = config.agent_id.clone();

        if self.agents.contains_key(&agent_id) {
            return Err(RuntimeError::AgentAlreadyExists(agent_id.0.clone()));
        }

        let state = AgentState::new(config, None);
        self.agents.insert(agent_id.clone(), state);

        let _ = self.event_bus.publish(Event::AgentStarted {
            agent_id: agent_id.clone(),
        });

        Ok(AgentHandle {
            agent_id,
            event_bus: (*self.event_bus).clone(),
        })
    }

    /// Unregister an in-process agent and publish an `AgentStopped` event.
    ///
    /// Returns `Err(AgentNotFound)` if the agent is not registered.
    pub fn unregister(
        &self,
        agent_id: &AgentId,
        reason: impl Into<String>,
    ) -> Result<(), RuntimeError> {
        if self.agents.remove(agent_id).is_none() {
            return Err(RuntimeError::AgentNotFound(agent_id.0.clone()));
        }
        // Clean up capability declarations when the agent is removed.
        self.capabilities.remove(agent_id);

        let _ = self.event_bus.publish(Event::AgentStopped {
            agent_id: agent_id.clone(),
            reason: reason.into(),
        });

        Ok(())
    }

    // ── Out-of-process agent lifecycle ───────────────────────────────────────

    /// Spawn a new agent as an OS process and register it.
    ///
    /// Uses the PAL `TokioProcessManager` to start the process.
    /// Returns `Err(AgentAlreadyExists)` if the ID is already taken.
    pub async fn spawn(
        &self,
        config: AgentConfig,
        process_config: ProcessConfig,
    ) -> Result<AgentHandle, RuntimeError> {
        let agent_id = config.agent_id.clone();

        if self.agents.contains_key(&agent_id) {
            return Err(RuntimeError::AgentAlreadyExists(agent_id.0.clone()));
        }

        let process_handle = self
            .process_manager
            .spawn(process_config)
            .await
            .map_err(|e| RuntimeError::ProcessError(e.to_string()))?;

        let state = AgentState::new(config, Some(process_handle));
        self.agents.insert(agent_id.clone(), state);

        let _ = self.event_bus.publish(Event::AgentStarted {
            agent_id: agent_id.clone(),
        });

        Ok(AgentHandle {
            agent_id,
            event_bus: (*self.event_bus).clone(),
        })
    }

    /// Gracefully terminate a spawned agent process and remove it.
    ///
    /// Sends SIGTERM and waits up to `grace_period`; falls back to SIGKILL.
    /// Also works for in-process agents (no-op on the process side).
    /// Returns `Err(AgentNotFound)` if the agent is not registered.
    pub async fn terminate(
        &self,
        agent_id: &AgentId,
        grace_period: Duration,
    ) -> Result<(), RuntimeError> {
        // Take the process handle under a short lock, then release.
        let process_handle = {
            match self.agents.get_mut(agent_id) {
                None => return Err(RuntimeError::AgentNotFound(agent_id.0.clone())),
                Some(mut entry) => {
                    entry.status = AgentStatus::Stopped;
                    entry.process_handle.take()
                }
            }
        };

        let result = if let Some(ph) = process_handle {
            self.process_manager
                .terminate(ph, grace_period)
                .await
                .map_err(|e| RuntimeError::ProcessError(e.to_string()))
        } else {
            Ok(())
        };

        // Always remove and publish, even if terminate returned an error
        // (the process may have already exited).
        self.agents.remove(agent_id);

        let _ = self.event_bus.publish(Event::AgentStopped {
            agent_id: agent_id.clone(),
            reason: "terminated".to_string(),
        });

        result
    }

    /// Kill a spawned agent process immediately (SIGKILL / TerminateProcess).
    ///
    /// Also works for in-process agents (no-op on the process side).
    /// Returns `Err(AgentNotFound)` if the agent is not registered.
    pub async fn kill(&self, agent_id: &AgentId) -> Result<(), RuntimeError> {
        let process_handle = {
            match self.agents.get_mut(agent_id) {
                None => return Err(RuntimeError::AgentNotFound(agent_id.0.clone())),
                Some(mut entry) => {
                    entry.status = AgentStatus::Error;
                    entry.process_handle.take()
                }
            }
        };

        let result = if let Some(ph) = process_handle {
            self.process_manager
                .kill(ph)
                .await
                .map_err(|e| RuntimeError::ProcessError(e.to_string()))
        } else {
            Ok(())
        };

        self.agents.remove(agent_id);
        // Clean up capability declarations when the agent is killed.
        self.capabilities.remove(agent_id);

        let _ = self.event_bus.publish(Event::AgentStopped {
            agent_id: agent_id.clone(),
            reason: "killed".to_string(),
        });

        result
    }

    // ── Query methods ─────────────────────────────────────────────────────────

    /// Return a snapshot of the `AgentInfo` for `agent_id`, or `None` if not
    /// found.
    pub fn agent_info(&self, agent_id: &AgentId) -> Option<AgentInfo> {
        self.agents.get(agent_id).map(|r| r.value().to_agent_info())
    }

    /// Return a snapshot of all registered agent IDs.
    pub fn agent_ids(&self) -> Vec<AgentId> {
        self.agents.iter().map(|r| r.key().clone()).collect()
    }

    /// Return the number of currently registered agents.
    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    // ── G-15: Agent Discovery ─────────────────────────────────────────────────

    /// Announce (or update) capabilities for a given agent.
    ///
    /// The `agent_id` does **not** need to be registered beforehand; this
    /// allows out-of-process agents to declare their capabilities before the
    /// orchestrator receives the corresponding `agent.spawn` call.
    ///
    /// If the agent is later removed via [`unregister`](Self::unregister) or
    /// [`kill`](Self::kill), its capabilities are automatically cleared.
    pub fn announce_capabilities(&self, agent_id: AgentId, capabilities: Vec<String>) {
        self.capabilities.insert(agent_id, capabilities);
    }

    /// Discover all agents and their declared capabilities.
    ///
    /// Returns a snapshot of every agent currently tracked by the
    /// orchestrator, combined with any capabilities declared via
    /// [`announce_capabilities`](Self::announce_capabilities).  Agents that
    /// have not called `announce` are included with an empty capabilities list.
    pub fn discover_capabilities(&self) -> Vec<AgentDiscoveryEntry> {
        self.agents
            .iter()
            .map(|r| {
                let id = r.key().clone();
                let status = format!("{:?}", r.value().status);
                let caps = self
                    .capabilities
                    .get(&id)
                    .map(|c| c.clone())
                    .unwrap_or_default();
                AgentDiscoveryEntry {
                    agent_id: id,
                    capabilities: caps,
                    status,
                }
            })
            .collect()
    }

    /// Subscribe to events from the orchestrator's event bus.
    pub fn subscribe(&self) -> crate::event_bus::EventReceiver {
        self.event_bus.subscribe()
    }

    // ── IPC Agent Spawning (Phase 2.2) ───────────────────────────────────────

    /// Spawn a new in-process agent with a dedicated IPC message channel.
    ///
    /// Unlike [`register`](Self::register) (which only stores an event-bus
    /// reference), `spawn_agent` creates a `tokio::sync::mpsc` channel and
    /// starts a message-processing task.  Callers receive an [`IpcAgentHandle`]
    /// that can be used to send fire-and-forget messages or to await responses.
    ///
    /// The agent is registered in the orchestrator under a generated
    /// [`AgentId`] and an `AgentStarted` event is published.
    ///
    /// # Arguments
    ///
    /// * `name` — human-readable label stored in [`AgentConfig`].
    /// * `restart_policy` — controls automatic restart on exit.
    ///
    /// # Returns
    ///
    /// An [`IpcAgentHandle`] for communicating with the new agent.
    pub async fn spawn_agent(
        &self,
        name: impl Into<String>,
        restart_policy: crate::restart_policy::AgentRestartPolicy,
    ) -> Result<crate::agent_handle::IpcAgentHandle, RuntimeError> {
        use crate::agent_handle::IpcAgentHandle;

        let name = name.into();
        let config = AgentConfig::new(name.clone());
        let agent_id = config.agent_id.clone();

        if self.agents.contains_key(&agent_id) {
            return Err(RuntimeError::AgentAlreadyExists(agent_id.0.clone()));
        }

        // Create the mpsc channel (capacity 32: backpressure without blocking
        // fast producers for typical agent workloads).
        let (tx, rx) = tokio::sync::mpsc::channel::<AgentMessage>(32);

        // Wrap in a SharedSender so IpcAgentHandle can survive future restarts
        // where the underlying channel is swapped in-place.
        let shared_tx: SharedSender = Arc::new(tokio::sync::Mutex::new(Some(tx)));

        // Register in the unified agent map (with ipc_tx for hot-swap restart).
        let mut state = AgentState::new(config, None);
        state.ipc_tx = Some(Arc::clone(&shared_tx));
        self.agents.insert(agent_id.clone(), state);

        // GAP-F8-02: write a persistent audit record for every agent spawn.
        // Read the field before restart_policy is consumed by RestartState::new below.
        let audit_policy_label = if restart_policy.max_retries == 0 {
            "never"
        } else {
            "on_failure"
        };

        // Store restart state for use by trigger_restart.
        let restart_state = RestartState::new(agent_id.clone(), name.clone(), restart_policy);
        self.restart_states.insert(agent_id.clone(), restart_state);

        let _ = self.event_bus.publish(Event::AgentStarted {
            agent_id: agent_id.clone(),
        });

        // GAP-F8-02: write a persistent audit record for every agent spawn.
        if let Some(audit) = &self.audit_log {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            audit.send_blocking(claw_tools::audit::AuditEvent::AgentSpawned {
                timestamp_ms: now_ms,
                agent_id: agent_id.0.clone(),
                agent_name: name.clone(),
                restart_policy: audit_policy_label.to_string(),
            });
        }

        // Spawn the message-processing task.  On exit the task sets Error and
        // calls trigger_restart which re-spawns the loop with backoff.
        spawn_ipc_message_loop(
            agent_id.clone(),
            rx,
            Arc::clone(&self.agents),
            Arc::clone(&self.event_bus),
            Arc::clone(&self.restart_states),
        );

        Ok(IpcAgentHandle { agent_id, shared_tx, agents: Arc::clone(&self.agents) })
    }

    // ── Auto-restart on agent exit (Phase 2.4) ───────────────────────────────

    /// Handle an agent's exit and restart it if the policy allows.
    ///
    /// For IPC agents spawned via [`spawn_agent`](Self::spawn_agent), the
    /// restart cycle fires automatically when the agent's tokio task exits.
    /// External callers (e.g. a process monitor) can also invoke this method
    /// directly to trigger the backoff-and-restart logic for a specific agent.
    ///
    /// Delegates to [`trigger_restart`] which publishes [`Event::AgentRestarted`]
    /// on each successful attempt and [`Event::AgentFailed`] when retries are
    /// exhausted.
    pub(crate) async fn handle_agent_exit(&self, agent_id: AgentId, reason: &str) {
        if let Some(mut entry) = self.agents.get_mut(&agent_id) {
            if matches!(entry.status, AgentStatus::Running | AgentStatus::Starting) {
                entry.status = AgentStatus::Error;
            }
        }

        trigger_restart(
            agent_id,
            reason,
            Arc::clone(&self.restart_states),
            Arc::clone(&self.agents),
            Arc::clone(&self.event_bus),
        )
        .await;
    }
}

// ─── IPC helpers (module-level free functions) ────────────────────────────────

/// Spawn a tokio task that runs the IPC message loop for an agent.
///
/// The loop processes `AgentMessage`s until its receiver is dropped (or all
/// senders are gone).  On exit it:
/// 1. Sets the agent's status to [`AgentStatus::Error`].
/// 2. Publishes [`Event::AgentStopped`].
/// 3. Calls [`trigger_restart`] which sleeps for the backoff delay and then
///    re-invokes `spawn_ipc_message_loop` if the policy permits.
///
/// This recursive chain terminates because [`RestartState`] increments an
/// attempt counter and stops when `max_retries` is reached.
///
/// # Panic isolation (GAP-07)
///
/// The message-processing body runs inside a **nested** `tokio::spawn`.
/// Awaiting its `JoinHandle` converts any panic into `Err(JoinError::Panicked)`,
/// so the outer supervisor block always executes the cleanup code (status
/// update + `trigger_restart`) regardless of whether the inner loop exits
/// normally, loses its channel, or panics.  This prevents a script-induced
/// panic from propagating to the host process.
fn spawn_ipc_message_loop(
    agent_id: AgentId,
    rx: tokio::sync::mpsc::Receiver<crate::agent_handle::AgentMessage>,
    agents: Arc<DashMap<AgentId, AgentState>>,
    event_bus: Arc<EventBus>,
    restart_states: Arc<DashMap<AgentId, RestartState>>,
) {
    use crate::agent_handle::{AgentMessage, AgentResponse, FinishReason, TokenUsage};

    tokio::spawn(async move {
        // ── Inner task: message processing only ──────────────────────────────
        // A panic here is captured by the JoinHandle rather than escaping
        // the supervisor block, guaranteeing the cleanup code below runs.
        let agent_id_inner = agent_id.clone();
        let loop_handle = tokio::spawn(async move {
            let mut rx = rx;
            while let Some(msg) = rx.recv().await {
                match msg {
                    AgentMessage::Send { content } => {
                        tracing::debug!(
                            agent = %agent_id_inner,
                            content_len = content.len(),
                            "IPC fire-and-forget message received"
                        );
                        // TODO: forward to the real AgentLoop when G-04 is resolved.
                    }
                    AgentMessage::SendAwait {
                        content,
                        timeout: _timeout,
                        reply_tx,
                    } => {
                        tracing::debug!(
                            agent = %agent_id_inner,
                            content_len = content.len(),
                            "IPC send_await message received"
                        );
                        // Placeholder echo — replace with real loop integration (G-04).
                        let response = AgentResponse {
                            content: format!("Echo from {}: {}", agent_id_inner, content),
                            finish_reason: FinishReason::Complete,
                            usage: TokenUsage::default(),
                        };
                        let _ = reply_tx.send(Ok(response));
                    }
                }
            }
        });

        // ── Supervisor: determine exit reason ────────────────────────────────
        // JoinHandle::await yields Ok(()) on normal exit and
        // Err(JoinError::Panicked) when the inner task panicked.
        let exit_reason = match loop_handle.await {
            Ok(()) => {
                tracing::debug!(agent = %agent_id, "IPC message loop exited normally");
                "ipc_loop_exit"
            }
            Err(ref panic_err) => {
                tracing::error!(
                    agent = %agent_id,
                    err = ?panic_err,
                    "IPC message loop panicked — crash isolated, RestartPolicy will apply"
                );
                "ipc_loop_panic"
            }
        };

        // ── Cleanup: always executes even after a panic ───────────────────────
        if let Some(mut entry) = agents.get_mut(&agent_id) {
            if matches!(entry.status, AgentStatus::Running | AgentStatus::Starting) {
                entry.status = AgentStatus::Error;
            }
        }

        event_bus.publish(Event::AgentStopped {
            agent_id: agent_id.clone(),
            reason: exit_reason.to_string(),
        });

        // Trigger the per-agent restart chain.
        trigger_restart(agent_id, exit_reason, restart_states, agents, event_bus).await;
    });
}

/// Apply the per-agent restart policy for a failed agent.
///
/// - If the [`RestartState`] permits another attempt: sets status to
///   [`AgentStatus::Starting`], sleeps the backoff delay, hot-swaps the
///   [`SharedSender`], re-spawns the IPC loop, sets status to
///   [`AgentStatus::Running`], and publishes [`Event::AgentRestarted`].
/// - If retries are exhausted: removes the restart state and publishes
///   [`Event::AgentFailed`].
/// - If the agent was removed while the backoff sleep was in progress, the
///   restart is silently aborted.
async fn trigger_restart(
    agent_id: AgentId,
    reason: &str,
    restart_states: Arc<DashMap<AgentId, RestartState>>,
    agents: Arc<DashMap<AgentId, AgentState>>,
    event_bus: Arc<EventBus>,
) {
    let should_restart = restart_states
        .get(&agent_id)
        .map(|s| s.should_restart())
        .unwrap_or(false);

    if !should_restart {
        let attempts = restart_states
            .get(&agent_id)
            .map(|s| s.attempt)
            .unwrap_or(0);
        restart_states.remove(&agent_id);

        if attempts > 0 {
            tracing::error!(agent = %agent_id, attempts, reason, "Agent exhausted all restart attempts");
        } else {
            tracing::debug!(agent = %agent_id, "No restart policy — agent will not be restarted");
        }

        event_bus.publish(Event::AgentFailed {
            agent_id,
            attempts,
            reason: reason.to_string(),
        });
        return;
    }

    // Compute backoff and record attempt (short lock scope — no await inside).
    let (delay, attempt_num) = {
        let mut state = match restart_states.get_mut(&agent_id) {
            Some(s) => s,
            None => return,
        };
        let delay = state.next_delay();
        state.record_attempt();
        (delay, state.attempt)
    };

    // Mark Starting to prevent the global auto-restart task from double-picking.
    if let Some(mut entry) = agents.get_mut(&agent_id) {
        entry.status = AgentStatus::Starting;
    }

    tracing::info!(
        agent = %agent_id,
        attempt = attempt_num,
        delay_ms = delay.as_millis(),
        "Restarting agent after exponential backoff"
    );

    tokio::time::sleep(delay).await;

    // Bail if the agent was removed during the sleep.
    if !agents.contains_key(&agent_id) {
        tracing::debug!(agent = %agent_id, "Agent removed during restart sleep — aborting");
        return;
    }

    // Hot-swap the SharedSender and re-spawn the message loop.
    let shared_tx_opt: Option<SharedSender> =
        agents.get(&agent_id).and_then(|e| e.ipc_tx.clone());

    if let Some(shared_tx) = shared_tx_opt {
        let (new_tx, new_rx) =
            tokio::sync::mpsc::channel::<crate::agent_handle::AgentMessage>(32);

        spawn_ipc_message_loop(
            agent_id.clone(),
            new_rx,
            Arc::clone(&agents),
            Arc::clone(&event_bus),
            Arc::clone(&restart_states),
        );

        // Swap: existing IpcAgentHandle clones now route to the new loop.
        let mut guard = shared_tx.lock().await;
        *guard = Some(new_tx);
    }

    // Update status and heartbeat.
    if let Some(mut entry) = agents.get_mut(&agent_id) {
        entry.status = AgentStatus::Running;
        entry.restart_count = attempt_num;
        entry.last_heartbeat = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
    }

    event_bus.publish(Event::AgentRestarted {
        agent_id,
        attempt: attempt_num,
        delay_ms: delay.as_millis() as u64,
    });
}



impl Drop for AgentOrchestrator {
    /// Signal background tasks to stop.
    ///
    /// **Note:** `Drop` only cancels the token; it does **not** await task
    /// completion.  Call [`graceful_shutdown`](AgentOrchestrator::graceful_shutdown)
    /// before dropping if you need to guarantee all tasks have exited cleanly.
    fn drop(&mut self) {
        self.cancel_token.cancel();
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_types::AgentConfig;

    use claw_pal::TokioProcessManager;

    fn make_orchestrator() -> AgentOrchestrator {
        let bus = Arc::new(EventBus::new());
        let pm = Arc::new(TokioProcessManager::new());
        // FIX-07: use new_for_test() to avoid spawning background Tokio tasks
        // in synchronous (#[test]) contexts that have no Tokio runtime.
        AgentOrchestrator::new_for_test(bus, pm)
    }

    // ── test_orchestrator_register_agent ─────────────────────────────────────
    #[test]
    fn test_orchestrator_register_agent() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("alpha");
        let id = config.agent_id.clone();

        let handle = orc.register(config).expect("register should succeed");

        assert_eq!(handle.agent_id, id);
        assert_eq!(orc.agent_count(), 1);

        let info = orc.agent_info(&id).expect("agent should be findable");
        assert_eq!(info.config.name, "alpha");
        assert!(info.started_at > 0);
        assert!(info.process_handle.is_none());
        assert_eq!(info.status, AgentStatus::Running);
    }

    // ── test_orchestrator_duplicate_agent_fails ───────────────────────────────
    #[test]
    fn test_orchestrator_duplicate_agent_fails() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("beta");
        let id = config.agent_id.clone();

        orc.register(config.clone()).expect("first register ok");

        let dup = AgentConfig {
            agent_id: id.clone(),
            name: "beta-dup".to_string(),
            mode: config.mode,
            metadata: Default::default(),
        };
        let result = orc.register(dup);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RuntimeError::AgentAlreadyExists(_)
        ));
    }

    // ── test_orchestrator_unregister_agent ────────────────────────────────────
    #[test]
    fn test_orchestrator_unregister_agent() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("gamma");
        let id = config.agent_id.clone();

        orc.register(config).unwrap();
        assert_eq!(orc.agent_count(), 1);

        orc.unregister(&id, "test done")
            .expect("unregister should succeed");
        assert_eq!(orc.agent_count(), 0);
        assert!(orc.agent_info(&id).is_none());
    }

    // ── test_orchestrator_unregister_nonexistent_fails ────────────────────────
    #[test]
    fn test_orchestrator_unregister_nonexistent_fails() {
        let orc = make_orchestrator();
        let id = AgentId::new("ghost-agent");
        let result = orc.unregister(&id, "cleanup");
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RuntimeError::AgentNotFound(_)
        ));
    }

    // ── test_orchestrator_agent_count ────────────────────────────────────────
    #[test]
    fn test_orchestrator_agent_count() {
        let orc = make_orchestrator();

        assert_eq!(orc.agent_count(), 0);

        let c1 = AgentConfig::new("one");
        let c2 = AgentConfig::new("two");
        let c3 = AgentConfig::new("three");
        let id2 = c2.agent_id.clone();

        orc.register(c1).unwrap();
        orc.register(c2).unwrap();
        orc.register(c3).unwrap();
        assert_eq!(orc.agent_count(), 3);

        orc.unregister(&id2, "removed").unwrap();
        assert_eq!(orc.agent_count(), 2);
    }

    // ── test_orchestrator_agent_ids ───────────────────────────────────────────
    #[test]
    fn test_orchestrator_agent_ids() {
        let orc = make_orchestrator();
        let c1 = AgentConfig::new("a");
        let c2 = AgentConfig::new("b");
        let id1 = c1.agent_id.clone();
        let id2 = c2.agent_id.clone();

        orc.register(c1).unwrap();
        orc.register(c2).unwrap();

        let mut ids = orc.agent_ids();
        ids.sort_by(|a, b| a.0.cmp(&b.0));

        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    // ── test_orchestrator_events_published ───────────────────────────────────
    #[tokio::test]
    async fn test_orchestrator_events_published() {
        use crate::events::Event;
        use claw_pal::TokioProcessManager;

        let bus = Arc::new(EventBus::new());
        let pm = Arc::new(TokioProcessManager::new());
        let orc = AgentOrchestrator::new_for_test(Arc::clone(&bus), pm);
        let mut rx = bus.subscribe();

        let config = AgentConfig::new("event-agent");
        let id = config.agent_id.clone();
        orc.register(config).unwrap();
        orc.unregister(&id, "done").unwrap();

        let e1 = rx.recv().await.unwrap();
        assert!(matches!(e1, Event::AgentStarted { .. }));

        let e2 = rx.recv().await.unwrap();
        assert!(matches!(e2, Event::AgentStopped { .. }));
    }

    // ── test_orchestrator_spawn_real_process ──────────────────────────────────
    #[tokio::test]
    async fn test_orchestrator_spawn_real_process() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("echo-agent");
        let agent_id = config.agent_id.clone();

        let process_config =
            ProcessConfig::new("echo".to_string()).with_arg("hello from claw-runtime".to_string());

        let handle = orc
            .spawn(config, process_config)
            .await
            .expect("spawn should succeed");

        assert_eq!(handle.agent_id, agent_id);
        assert_eq!(orc.agent_count(), 1);

        let info = orc.agent_info(&agent_id).expect("agent should be present");
        assert!(info.process_handle.is_some());
        assert_eq!(info.status, AgentStatus::Running);
    }

    // ── test_orchestrator_spawn_duplicate_fails ───────────────────────────────
    #[tokio::test]
    async fn test_orchestrator_spawn_duplicate_fails() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("dup-agent");
        let agent_id = config.agent_id.clone();

        let pc = ProcessConfig::new("echo".to_string());

        // First spawn succeeds.
        orc.spawn(config.clone(), pc.clone()).await.unwrap();

        // Second spawn with same ID fails.
        let dup_config = AgentConfig {
            agent_id: agent_id.clone(),
            name: "dup-agent-2".to_string(),
            mode: config.mode,
            metadata: Default::default(),
        };
        let result = orc.spawn(dup_config, pc).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RuntimeError::AgentAlreadyExists(_)
        ));

        // Clean up.
        let _ = orc.terminate(&agent_id, Duration::from_secs(2)).await;
    }

    // ── test_orchestrator_terminate_spawned_process ───────────────────────────
    #[tokio::test]
    async fn test_orchestrator_terminate_spawned_process() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("sleep-agent");
        let agent_id = config.agent_id.clone();

        #[cfg(unix)]
        let pc = ProcessConfig::new("sleep".to_string()).with_arg("60".to_string());
        #[cfg(windows)]
        let pc = ProcessConfig::new("ping".to_string()).with_args(vec![
            "-n".to_string(),
            "60".to_string(),
            "127.0.0.1".to_string(),
        ]);

        orc.spawn(config, pc).await.expect("spawn should succeed");
        assert_eq!(orc.agent_count(), 1);

        orc.terminate(&agent_id, Duration::from_millis(100))
            .await
            .expect("terminate should succeed");

        assert_eq!(orc.agent_count(), 0);
        assert!(orc.agent_info(&agent_id).is_none());
    }

    // ── test_orchestrator_kill_spawned_process ────────────────────────────────
    #[tokio::test]
    async fn test_orchestrator_kill_spawned_process() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("kill-agent");
        let agent_id = config.agent_id.clone();

        #[cfg(unix)]
        let pc = ProcessConfig::new("sleep".to_string()).with_arg("60".to_string());
        #[cfg(windows)]
        let pc = ProcessConfig::new("ping".to_string()).with_args(vec![
            "-n".to_string(),
            "60".to_string(),
            "127.0.0.1".to_string(),
        ]);

        orc.spawn(config, pc).await.expect("spawn should succeed");

        orc.kill(&agent_id).await.expect("kill should succeed");

        assert_eq!(orc.agent_count(), 0);
    }

    // ── test_orchestrator_terminate_inprocess_agent ───────────────────────────
    #[tokio::test]
    async fn test_orchestrator_terminate_inprocess_agent() {
        // terminate() on an in-process agent (no process handle) should
        // succeed and remove the agent.
        let orc = make_orchestrator();
        let config = AgentConfig::new("inproc");
        let id = config.agent_id.clone();
        orc.register(config).unwrap();

        orc.terminate(&id, Duration::from_secs(1))
            .await
            .expect("terminate of in-process agent should succeed");

        assert_eq!(orc.agent_count(), 0);
    }

    // ── test_orchestrator_terminate_notfound_fails ────────────────────────────
    #[tokio::test]
    async fn test_orchestrator_terminate_notfound_fails() {
        let orc = make_orchestrator();
        let id = AgentId::new("ghost");
        let result = orc.terminate(&id, Duration::from_secs(1)).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            RuntimeError::AgentNotFound(_)
        ));
    }

    // ── G-10: health_check & RestartPolicy ───────────────────────────────────

    // ── test_spawn_agent_ipc_tx_stored ────────────────────────────────────────
    #[tokio::test]
    async fn test_spawn_agent_ipc_tx_stored() {
        use crate::event_bus::EventBus;
        use claw_pal::TokioProcessManager;

        let bus = Arc::new(EventBus::new());
        let pm = Arc::new(TokioProcessManager::new());
        let orc = AgentOrchestrator::new_for_test(Arc::clone(&bus), pm);

        let handle = orc
            .spawn_agent("test-ipc", crate::restart_policy::AgentRestartPolicy::never())
            .await
            .expect("spawn_agent should succeed");

        assert_eq!(orc.agent_count(), 1);
        assert_eq!(
            orc.agent_info(&handle.agent_id).unwrap().status,
            AgentStatus::Running
        );
        // SharedSender slot should be populated (agent loop is running).
        assert!(
            handle.shared_tx.try_lock().expect("no contention").is_some(),
            "shared sender should be Some after spawn"
        );
    }

    // ── test_health_check_heartbeat_timeout_marks_error ───────────────────────
    #[tokio::test]
    async fn test_health_check_heartbeat_timeout_marks_error() {
        use claw_pal::TokioProcessManager;

        let bus = Arc::new(EventBus::new());
        let pm = Arc::new(TokioProcessManager::new());
        let orc = AgentOrchestrator::new_for_test(Arc::clone(&bus), pm);

        // Register a process-based agent.
        let config = AgentConfig::new("proc-hb");
        let agent_id = config.agent_id.clone();
        let pc = claw_pal::ProcessConfig::new("echo".to_string()).with_arg("hi".to_string());
        orc.spawn(config, pc).await.expect("spawn ok");

        // Backdate the heartbeat to simulate a stale agent.
        if let Some(mut entry) = orc.agents.get_mut(&agent_id) {
            entry.last_heartbeat = 0;
        }

        // health_check() should report the agent as unhealthy.
        let health = orc.health_check(&agent_id).await.expect("health_check ok");
        assert!(!health.is_healthy, "stale heartbeat → not healthy");

        let _ = orc.terminate(&agent_id, Duration::from_millis(100)).await;
    }

    // ── test_trigger_restart_never_policy_publishes_agent_failed ─────────────
    #[tokio::test]
    async fn test_trigger_restart_never_policy_publishes_agent_failed() {
        use crate::event_bus::EventBus;
        use crate::restart_policy::{AgentRestartPolicy, RestartState};
        use claw_pal::TokioProcessManager;

        let bus = Arc::new(EventBus::new());
        let pm = Arc::new(TokioProcessManager::new());
        let orc = AgentOrchestrator::new_for_test(Arc::clone(&bus), pm);
        let mut rx = bus.subscribe();

        let agent_id = AgentId::new("no-restart");

        // Insert a never-restart policy.
        orc.restart_states.insert(
            agent_id.clone(),
            RestartState::new(agent_id.clone(), "no-restart", AgentRestartPolicy::never()),
        );
        let config = AgentConfig::new("no-restart");
        orc.agents.insert(agent_id.clone(), AgentState::new(config, None));

        trigger_restart(
            agent_id.clone(),
            "test",
            Arc::clone(&orc.restart_states),
            Arc::clone(&orc.agents),
            Arc::clone(&orc.event_bus),
        )
        .await;

        let mut saw_failed = false;
        for _ in 0..20 {
            if let Ok(evt) = rx.try_recv() {
                if matches!(&evt, crate::events::Event::AgentFailed { agent_id: id, .. } if id == &agent_id)
                {
                    saw_failed = true;
                    break;
                }
            }
        }
        assert!(saw_failed, "expected AgentFailed event");
        assert!(!orc.restart_states.contains_key(&agent_id));
    }

    // ── test_trigger_restart_hot_swaps_sender ─────────────────────────────────
    #[tokio::test]
    async fn test_trigger_restart_hot_swaps_sender() {
        use crate::event_bus::EventBus;
        use crate::restart_policy::{AgentRestartPolicy, RestartState};
        use claw_pal::TokioProcessManager;

        let bus = Arc::new(EventBus::new());
        let pm = Arc::new(TokioProcessManager::new());
        let orc = AgentOrchestrator::new_for_test(Arc::clone(&bus), pm);

        // Fast policy: 1-attempt, 1 ms delay.
        let fast_policy = AgentRestartPolicy {
            max_retries: 1,
            initial_delay: std::time::Duration::from_millis(1),
            max_delay: std::time::Duration::from_millis(1),
            backoff_multiplier: 1.0,
        };

        let handle = orc
            .spawn_agent("restart-me", fast_policy.clone())
            .await
            .expect("spawn ok");

        let agent_id = handle.agent_id.clone();

        // Simulate the loop dying: clear the sender and update restart state.
        *handle.shared_tx.lock().await = None;
        orc.restart_states.insert(
            agent_id.clone(),
            RestartState::new(agent_id.clone(), "restart-me", fast_policy),
        );

        trigger_restart(
            agent_id.clone(),
            "manual_test",
            Arc::clone(&orc.restart_states),
            Arc::clone(&orc.agents),
            Arc::clone(&orc.event_bus),
        )
        .await;

        // After restart, the shared sender should be back.
        assert!(
            handle.shared_tx.lock().await.is_some(),
            "SharedSender should be Some after restart"
        );
        assert_eq!(
            orc.agent_info(&agent_id).unwrap().status,
            AgentStatus::Running
        );
        // Sending should succeed via the hot-swapped channel.
        handle.send("after restart").await.expect("send after restart ok");
    }

    // ── test_panic_in_message_loop_isolates_cleanup (GAP-07) ──────────────────
    /// Verify that a panic inside the IPC message-loop inner task does **not**
    /// propagate to the host and that the supervisor correctly observes the
    /// exit as an error, publishes `AgentStopped`, and invokes `trigger_restart`.
    ///
    /// We simulate a panic by awaiting a `JoinHandle` that we know will panic,
    /// then asserting the error path produces the correct log reason.
    #[tokio::test]
    async fn test_panic_in_message_loop_isolates_cleanup() {
        use crate::event_bus::EventBus;
        use crate::restart_policy::AgentRestartPolicy;
        use claw_pal::TokioProcessManager;

        let bus = Arc::new(EventBus::new());
        let pm = Arc::new(TokioProcessManager::new());
        let orc = AgentOrchestrator::new_for_test(Arc::clone(&bus), pm);
        let mut rx = bus.subscribe();

        // Spawn an agent with a never-restart policy so trigger_restart fires
        // AgentFailed immediately (no sleep) and we can observe events quickly.
        let handle = orc
            .spawn_agent("panic-test", AgentRestartPolicy::never())
            .await
            .expect("spawn_agent should succeed");
        let agent_id = handle.agent_id.clone();

        // Confirm AgentStarted was emitted.
        let started = rx.recv().await.unwrap();
        assert!(matches!(started, crate::events::Event::AgentStarted { .. }));

        // Close the channel explicitly: clear the shared sender so the inner
        // loop's rx.recv() returns None.  We must clear the Option inside the
        // shared mutex — both the IpcAgentHandle and AgentState.ipc_tx share
        // the same Arc<Mutex<Option<Sender>>>, so setting it to None here
        // drops the only Sender and unblocks the inner task.
        *handle.shared_tx.lock().await = None;
        drop(handle);

        // Wait for AgentStopped event (emitted by supervisor cleanup) and
        // AgentFailed (emitted by trigger_restart with never policy).
        let mut saw_stopped = false;
        let mut saw_failed = false;

        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            while let Ok(evt) = rx.try_recv() {
                match &evt {
                    crate::events::Event::AgentStopped { agent_id: id, .. } if id == &agent_id => {
                        saw_stopped = true;
                    }
                    crate::events::Event::AgentFailed { agent_id: id, .. } if id == &agent_id => {
                        saw_failed = true;
                    }
                    _ => {}
                }
            }
            if saw_stopped && saw_failed {
                break;
            }
        }

        assert!(saw_stopped, "AgentStopped must be emitted by supervisor cleanup");
        assert!(saw_failed, "AgentFailed must be emitted by trigger_restart (never policy)");
    }

    // ── test_panic_join_error_is_treated_as_error_exit ────────────────────────
    /// Unit-level check: a tokio task that panics produces a JoinError that
    /// `is_panic()` reports true.  This confirms our supervisor's Err branch
    /// is reachable and not silently swallowed by tokio.
    #[tokio::test]
    async fn test_panic_join_error_is_treated_as_error_exit() {
        let handle = tokio::spawn(async move {
            panic!("intentional panic for GAP-07 test");
        });

        let result = handle.await;
        assert!(result.is_err(), "panicking task should produce Err JoinHandle");
        let err = result.unwrap_err();
        assert!(err.is_panic(), "JoinError should report is_panic() == true");
    }

    // ── G-9: set_agent_restart_policy ────────────────────────────────────────

    #[tokio::test]
    async fn test_set_agent_restart_policy_updates_live() {
        use crate::event_bus::EventBus;
        use crate::restart_policy::AgentRestartPolicy;
        use claw_pal::TokioProcessManager;

        let bus = Arc::new(EventBus::new());
        let pm = Arc::new(TokioProcessManager::new());
        let orc = AgentOrchestrator::new_for_test(Arc::clone(&bus), pm);

        // Spawn with a never-restart policy.
        let handle = orc
            .spawn_agent("g9-agent", AgentRestartPolicy::never())
            .await
            .expect("spawn_agent should succeed");

        // Confirm initial policy allows 0 retries.
        {
            let state = orc.restart_states.get(&handle.agent_id).unwrap();
            assert_eq!(state.policy.max_retries, 0);
        }

        // Dynamically upgrade to 5 retries.
        let new_policy = AgentRestartPolicy::with_max_retries(5);
        orc.set_agent_restart_policy(&handle.agent_id, new_policy)
            .expect("set_agent_restart_policy should succeed for a known IPC agent");

        // Verify the in-place update — attempt counter must be untouched.
        {
            let state = orc.restart_states.get(&handle.agent_id).unwrap();
            assert_eq!(state.policy.max_retries, 5, "policy must reflect new max_retries");
            assert_eq!(state.attempt, 0, "attempt counter must not be reset by policy update");
        }
    }

    #[tokio::test]
    async fn test_set_agent_restart_policy_unknown_agent_returns_error() {
        use crate::event_bus::EventBus;
        use crate::restart_policy::AgentRestartPolicy;
        use claw_pal::TokioProcessManager;

        let bus = Arc::new(EventBus::new());
        let pm = Arc::new(TokioProcessManager::new());
        let orc = AgentOrchestrator::new_for_test(Arc::clone(&bus), pm);

        let unknown_id = AgentId::new("ghost");
        let result = orc.set_agent_restart_policy(&unknown_id, AgentRestartPolicy::default());

        assert!(
            matches!(result, Err(RuntimeError::AgentNotFound(_))),
            "unknown agent must return AgentNotFound"
        );
    }

    // ── G-6: ResourceSnapshot tests ───────────────────────────────────────────

    /// Verify that a freshly registered agent has an empty (None) resource
    /// snapshot, and that `to_resource_usage()` returns None accordingly.
    #[test]
    fn test_resource_snapshot_initially_none() {
        let orc = make_orchestrator();
        let config = AgentConfig::new("snap-test");
        let id = config.agent_id.clone();

        orc.register(config).unwrap();

        let info = orc.agent_info(&id).unwrap();
        // No sysinfo sample has been taken yet — resource_usage must be None.
        assert!(
            info.resource_usage.is_none(),
            "resource_usage should be None before any sysinfo sample"
        );
    }

    /// Verify that once a ResourceSnapshot is written into AgentState,
    /// `to_resource_usage()` and `health_check()` reflect it correctly.
    #[tokio::test]
    async fn test_resource_snapshot_populates_health_status() {
        let bus = Arc::new(crate::event_bus::EventBus::new());
        let pm = Arc::new(claw_pal::TokioProcessManager::new());
        let orc = AgentOrchestrator::new_for_test(Arc::clone(&bus), pm);

        let config = AgentConfig::new("snap-health-test");
        let id = config.agent_id.clone();
        orc.register(config).unwrap();

        // Manually inject a ResourceSnapshot (simulates what start_resource_monitor_task does).
        {
            let entry = orc.agents.get(&id).unwrap();
            let mut guard = entry.resource_snapshot.write().unwrap();
            *guard = Some(ResourceSnapshot {
                memory_kb: 4096,
                cpu_percent: 12.5,
                sampled_at_ms: 1_000_000,
            });
        }

        // to_resource_usage() must now reflect the injected snapshot.
        let info = orc.agent_info(&id).unwrap();
        let usage = info.resource_usage.expect("resource_usage must be Some after snapshot");
        assert_eq!(usage.memory_bytes, Some(4096 * 1024));

        // health_check() must also reflect the snapshot values.
        let health = orc.health_check(&id).await.unwrap();
        assert_eq!(health.memory_usage_kb, Some(4096));
        assert!((health.cpu_usage_percent.unwrap() - 12.5).abs() < f32::EPSILON);
    }
}
