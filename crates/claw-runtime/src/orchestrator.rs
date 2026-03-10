use claw_pal::traits::ProcessManager;
use claw_pal::ProcessConfig;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;

use crate::{
    agent_types::{AgentConfig, AgentHandle, AgentId, AgentInfo, AgentStatus},
    error::RuntimeError,
    event_bus::EventBus,
    events::Event,
};

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
}

impl AgentState {
    fn new(config: AgentConfig, process_handle: Option<claw_pal::ProcessHandle>) -> Self {
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
        }
    }

    /// Convert to the public `AgentInfo` view (for backward-compatible API).
    fn to_agent_info(&self) -> AgentInfo {
        AgentInfo {
            config: self.config.clone(),
            started_at: self.started_at,
            process_handle: self.process_handle.clone(),
            status: self.status,
        }
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
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            heartbeat_timeout_ms: 30_000,
            health_check_interval_secs: 10,
        }
    }
}

// ─── AgentOrchestrator ────────────────────────────────────────────────────────

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
    cancel_token: tokio_util::sync::CancellationToken,
    /// Orchestrator behaviour configuration.
    config: OrchestratorConfig,
    /// Per-agent restart tracking for the IPC spawn path (Phase 2.4).
    restart_states: Arc<DashMap<AgentId, crate::restart_policy::RestartState>>,
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
            cancel_token: tokio_util::sync::CancellationToken::new(),
            config: OrchestratorConfig::default(),
            restart_states: Arc::new(DashMap::new()),
        }
    }

    /// Start background maintenance tasks (heartbeat monitor, cleanup, etc.).
    ///
    /// Call this after creating the orchestrator and before registering agents.
    pub fn start(&self) {
        self.start_health_check_task();
        self.start_auto_restart_task();
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
            cancel_token: tokio_util::sync::CancellationToken::new(),
            config: OrchestratorConfig::default(),
            restart_states: Arc::new(DashMap::new()),
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

        // Copy last known metrics from previously stored health
        if let Some(ref health) = entry.health {
            status.memory_usage_kb = health.memory_usage_kb;
            status.cpu_usage_percent = health.cpu_usage_percent;
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
            // TriggerHeartbeat and Custom are handled above
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
        }

        Ok(())
    }

    // ── Internal Background Tasks ───────────────────────────────────────────────

    /// Start the health check background task.
    fn start_health_check_task(&self) {
        let agents = Arc::clone(&self.agents);
        let cancel_token = self.cancel_token.clone();
        let health_check_interval_secs = self.config.health_check_interval_secs;

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

                        // Collect running agent IDs first to avoid holding refs during mut access.
                        let agent_ids: Vec<AgentId> = agents.iter()
                            .filter(|e| e.value().status == AgentStatus::Running)
                            .map(|e| e.key().clone())
                            .collect();

                        for agent_id in agent_ids {
                            if let Some(mut entry) = agents.get_mut(&agent_id) {
                                // Auto-record heartbeat for healthy agents
                                if now.saturating_sub(entry.last_heartbeat) > 10_000 {
                                    entry.last_heartbeat = now;
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
    fn start_auto_restart_task(&self) {
        let agents = Arc::clone(&self.agents);
        let restart_policy = Arc::clone(&self.restart_policy);
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

                        // Collect agents that need restart.
                        let to_restart: Vec<AgentId> = agents.iter()
                            .filter(|e| {
                                (e.value().status == AgentStatus::Stopped
                                    || e.value().status == AgentStatus::Error)
                                    && e.value().restart_count < policy.max_restarts
                            })
                            .map(|e| e.key().clone())
                            .collect();

                        for agent_id in to_restart {
                            if let Some(mut entry) = agents.get_mut(&agent_id) {
                                // TODO: Implement actual restart logic
                                // This would spawn a new process with the same config
                                entry.restart_count += 1;
                            }

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
        use crate::agent_handle::{AgentMessage, AgentResponse, FinishReason, IpcAgentHandle, TokenUsage};
        use crate::restart_policy::RestartState;

        let name = name.into();
        let config = AgentConfig::new(name.clone());
        let agent_id = config.agent_id.clone();

        if self.agents.contains_key(&agent_id) {
            return Err(RuntimeError::AgentAlreadyExists(agent_id.0.clone()));
        }

        // Create the mpsc channel (capacity 32: backpressure without blocking
        // fast producers for typical agent workloads).
        let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentMessage>(32);

        // Register in the unified agent map.
        let state = AgentState::new(config, None);
        self.agents.insert(agent_id.clone(), state);

        // Store restart state for use by handle_agent_exit.
        let restart_state = RestartState::new(agent_id.clone(), name.clone(), restart_policy);
        self.restart_states.insert(agent_id.clone(), restart_state);

        let _ = self.event_bus.publish(Event::AgentStarted {
            agent_id: agent_id.clone(),
        });

        // Spawn the message-processing task.
        let agent_id_task = agent_id.clone();
        let event_bus_task = Arc::clone(&self.event_bus);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                match msg {
                    AgentMessage::Send { content } => {
                        tracing::debug!(
                            agent = %agent_id_task,
                            content_len = content.len(),
                            "IPC fire-and-forget message received"
                        );
                        // TODO: forward to the actual agent loop when wired up.
                    }
                    AgentMessage::SendAwait {
                        content,
                        timeout: _timeout,
                        reply_tx,
                    } => {
                        tracing::debug!(
                            agent = %agent_id_task,
                            content_len = content.len(),
                            "IPC send_await message received"
                        );
                        // Placeholder echo response — replace with real agent
                        // loop integration in a future milestone.
                        let response = AgentResponse {
                            content: format!("Echo from {}: {}", agent_id_task, content),
                            finish_reason: FinishReason::Complete,
                            usage: TokenUsage::default(),
                        };
                        // Ignore send error: caller may have timed out already.
                        let _ = reply_tx.send(Ok(response));
                    }
                }
            }
            // Channel closed — agent loop exited.
            tracing::debug!(agent = %agent_id_task, "IPC message loop exited");
            event_bus_task.publish(Event::AgentStopped {
                agent_id: agent_id_task,
                reason: "ipc_loop_exit".to_string(),
            });
        });

        Ok(IpcAgentHandle { agent_id, tx })
    }

    // ── Auto-restart on agent exit (Phase 2.4) ───────────────────────────────

    /// Handle an agent's exit and restart it if the policy allows.
    ///
    /// Called internally when an agent's process or task exits unexpectedly.
    /// Looks up the agent's [`AgentRestartPolicy`](crate::restart_policy::AgentRestartPolicy),
    /// waits for the computed backoff delay, then re-registers the agent.
    /// Publishes [`Event::AgentRestarted`] on each successful restart attempt
    /// and [`Event::AgentFailed`] when retries are exhausted.
    pub(crate) async fn handle_agent_exit(&self, agent_id: AgentId, reason: &str) {
        // Look up the restart state; if none exists, nothing to do.
        let should_restart = self
            .restart_states
            .get(&agent_id)
            .map(|s| s.should_restart())
            .unwrap_or(false);

        if !should_restart {
            // No restart — publish failure event and clean up restart state.
            let attempts = self
                .restart_states
                .get(&agent_id)
                .map(|s| s.attempt)
                .unwrap_or(0);
            self.restart_states.remove(&agent_id);

            self.event_bus.publish(Event::AgentFailed {
                agent_id,
                attempts,
                reason: reason.to_string(),
            });
            return;
        }

        // Compute backoff delay and record the attempt.
        let (delay, attempt_num) = {
            let mut state = match self.restart_states.get_mut(&agent_id) {
                Some(s) => s,
                None => return,
            };
            let delay = state.next_delay();
            state.record_attempt();
            (delay, state.attempt)
        };

        tracing::info!(
            agent = %agent_id,
            attempt = attempt_num,
            delay_ms = delay.as_millis(),
            "Scheduling agent restart with exponential backoff"
        );

        tokio::time::sleep(delay).await;

        // Re-register the agent as Running (re-uses existing config if present).
        if let Some(mut entry) = self.agents.get_mut(&agent_id) {
            entry.status = AgentStatus::Running;
            entry.restart_count = attempt_num;
        }

        self.event_bus.publish(Event::AgentRestarted {
            agent_id,
            attempt: attempt_num,
            delay_ms: delay.as_millis() as u64,
        });
    }
}

// ─── Drop ─────────────────────────────────────────────────────────────────────

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
}
