//! KernelServer implementation.
//!
//! Provides a JSON-RPC 2.0 server over local IPC for remote agent control.

use std::sync::Arc;

use tokio::net::UnixListener;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use claw_provider::traits::LLMProvider;

use claw_runtime::EventBus;
use claw_runtime::orchestrator::AgentOrchestrator;
use crate::channel_registry::ChannelRegistry;
use crate::error::ServerError;
use crate::global_skill_registry::GlobalSkillRegistry;
use crate::global_tool_registry::GlobalToolRegistry;
use crate::session::SessionManager;

/// Configuration for the LLM provider.
#[derive(Debug, Clone)]
pub enum ProviderConfig {
    /// Anthropic Claude provider.
    Anthropic {
        /// API key for authentication.
        api_key: String,
        /// Default model to use.
        default_model: String,
    },
    /// OpenAI provider.
    OpenAI {
        /// API key for authentication.
        api_key: String,
        /// Base URL for the API.
        base_url: String,
        /// Default model to use.
        default_model: String,
    },
    /// Ollama local provider.
    Ollama {
        /// Base URL for the API.
        base_url: String,
        /// Default model to use.
        default_model: String,
    },
    /// DeepSeek provider.
    DeepSeek {
        /// API key for authentication.
        api_key: String,
        /// Default model to use.
        default_model: String,
    },
    /// Moonshot provider.
    Moonshot {
        /// API key for authentication.
        api_key: String,
        /// Default model to use.
        default_model: String,
    },
    /// Google Gemini provider.
    #[cfg(feature = "gemini")]
    Gemini {
        /// API key for authentication.
        api_key: String,
        /// Default model to use.
        default_model: String,
    },
    /// Mistral AI provider.
    #[cfg(feature = "mistral")]
    Mistral {
        /// API key for authentication.
        api_key: String,
        /// Default model to use.
        default_model: String,
    },
    /// Azure OpenAI provider.
    #[cfg(feature = "azure-openai")]
    AzureOpenAI {
        /// API key for authentication.
        api_key: String,
        /// Azure resource name.
        resource_name: String,
        /// Azure deployment ID.
        deployment_id: String,
        /// Azure API version (default: "2024-02-01").
        api_version: String,
    },
    /// Dynamic provider selection (configured at runtime).
    Dynamic,
}

impl ProviderConfig {
    /// Returns the provider name.
    pub fn provider_name(&self) -> &'static str {
        match self {
            ProviderConfig::Anthropic { .. } => "anthropic",
            ProviderConfig::OpenAI { .. } => "openai",
            ProviderConfig::Ollama { .. } => "ollama",
            ProviderConfig::DeepSeek { .. } => "deepseek",
            ProviderConfig::Moonshot { .. } => "moonshot",
            #[cfg(feature = "gemini")]
            ProviderConfig::Gemini { .. } => "gemini",
            #[cfg(feature = "mistral")]
            ProviderConfig::Mistral { .. } => "mistral",
            #[cfg(feature = "azure-openai")]
            ProviderConfig::AzureOpenAI { .. } => "azure-openai",
            ProviderConfig::Dynamic => "dynamic",
        }
    }

    /// Returns the default model if configured.
    pub fn default_model(&self) -> Option<&str> {
        match self {
            ProviderConfig::Anthropic { default_model, .. } => Some(default_model),
            ProviderConfig::OpenAI { default_model, .. } => Some(default_model),
            ProviderConfig::Ollama { default_model, .. } => Some(default_model),
            ProviderConfig::DeepSeek { default_model, .. } => Some(default_model),
            ProviderConfig::Moonshot { default_model, .. } => Some(default_model),
            #[cfg(feature = "gemini")]
            ProviderConfig::Gemini { default_model, .. } => Some(default_model),
            #[cfg(feature = "mistral")]
            ProviderConfig::Mistral { default_model, .. } => Some(default_model),
            #[cfg(feature = "azure-openai")]
            ProviderConfig::AzureOpenAI { deployment_id, .. } => Some(deployment_id),
            ProviderConfig::Dynamic => None,
        }
    }

    /// Builds an `Arc<dyn LLMProvider>` from this configuration.
    pub fn build_provider(&self) -> Arc<dyn LLMProvider> {
        match self {
            #[cfg(feature = "anthropic")]
            ProviderConfig::Anthropic { api_key, default_model } => {
                Arc::new(claw_provider::AnthropicProvider::new(
                    api_key.clone(),
                    default_model.clone(),
                ))
            }
            #[cfg(feature = "openai")]
            ProviderConfig::OpenAI { api_key, base_url, default_model } => {
                Arc::new(
                    claw_provider::OpenAIProvider::new(api_key.clone(), default_model.clone())
                        .with_base_url(base_url.clone()),
                )
            }
            #[cfg(feature = "ollama")]
            ProviderConfig::Ollama { base_url, default_model } => {
                Arc::new(
                    claw_provider::OllamaProvider::new(default_model.clone())
                        .with_base_url(base_url.clone()),
                )
            }
            #[cfg(feature = "deepseek")]
            ProviderConfig::DeepSeek { api_key, default_model } => {
                Arc::new(claw_provider::DeepSeekProvider::new(
                    api_key.clone(),
                    default_model.clone(),
                ))
            }
            #[cfg(feature = "moonshot")]
            ProviderConfig::Moonshot { api_key, default_model } => {
                Arc::new(claw_provider::MoonshotProvider::new(
                    api_key.clone(),
                    default_model.clone(),
                ))
            }
            #[cfg(feature = "gemini")]
            ProviderConfig::Gemini { api_key, default_model } => {
                Arc::new(claw_provider::gemini::gemini_provider(api_key.clone(), default_model.clone()))
            }
            #[cfg(feature = "mistral")]
            ProviderConfig::Mistral { api_key, default_model } => {
                Arc::new(claw_provider::mistral::mistral_provider(api_key.clone(), default_model.clone()))
            }
            #[cfg(feature = "azure-openai")]
            ProviderConfig::AzureOpenAI { api_key, resource_name, deployment_id, api_version } => {
                Arc::new(claw_provider::azure_openai::azure_openai_provider(
                    resource_name.clone(),
                    deployment_id.clone(),
                    api_version.clone(),
                    api_key.clone(),
                ))
            }
            ProviderConfig::Dynamic => {
                // Dynamic: try Anthropic from env, fall back to Ollama.
                // The panic below is only reached when neither "anthropic" nor "ollama"
                // features are compiled in, so suppress the unreachable-code lint.
                #[cfg(feature = "anthropic")]
                if let Ok(p) = claw_provider::AnthropicProvider::from_env() {
                    return Arc::new(p);
                }
                #[cfg(feature = "ollama")]
                return Arc::new(claw_provider::OllamaProvider::new("llama3.2:latest"));
                #[cfg(not(any(feature = "anthropic", feature = "ollama")))]
                panic!("No provider feature enabled; cannot build a Dynamic provider");
            }
            // Fallback: if the matching feature is disabled, panic with a clear message.
            #[allow(unreachable_patterns)]
            _ => panic!(
                "Provider '{}' selected but the corresponding feature flag is not enabled",
                self.provider_name()
            ),
        }
    }
}

/// A registry of named LLM providers.
///
/// Supports a default provider plus optional named overrides for per-session routing.
pub struct ProviderRegistry {
    providers: dashmap::DashMap<String, std::sync::Arc<dyn claw_provider::traits::LLMProvider>>,
    default_name: String,
}

impl ProviderRegistry {
    /// Create a registry with a single default provider.
    pub fn new(name: impl Into<String>, provider: std::sync::Arc<dyn claw_provider::traits::LLMProvider>) -> Self {
        let name = name.into();
        let providers = dashmap::DashMap::new();
        providers.insert(name.clone(), provider);
        Self { providers, default_name: name }
    }

    /// Register an additional named provider.
    pub fn register(&self, name: impl Into<String>, provider: std::sync::Arc<dyn claw_provider::traits::LLMProvider>) {
        self.providers.insert(name.into(), provider);
    }

    /// Get a provider by name. Returns None if not found.
    pub fn get(&self, name: &str) -> Option<std::sync::Arc<dyn claw_provider::traits::LLMProvider>> {
        self.providers.get(name).map(|v| std::sync::Arc::clone(v.value()))
    }

    /// Get the default provider.
    pub fn default_provider(&self) -> std::sync::Arc<dyn claw_provider::traits::LLMProvider> {
        self.providers.get(&self.default_name)
            .map(|v| std::sync::Arc::clone(v.value()))
            .expect("default provider must always be present")
    }

    /// Get the default provider name.
    pub fn default_name(&self) -> &str {
        &self.default_name
    }

    /// List all registered provider names.
    pub fn names(&self) -> Vec<String> {
        self.providers.iter().map(|e| e.key().clone()).collect()
    }
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("default", &self.default_name)
            .field("count", &self.providers.len())
            .finish()
    }
}

/// Configuration for the KernelServer.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Path to the Unix socket for IPC.
    pub socket_path: String,
    /// Maximum number of concurrent sessions.
    pub max_sessions: usize,
    /// LLM provider configuration.
    pub provider_config: ProviderConfig,
    /// Optional port for the built-in HTTP webhook server.
    /// Set to Some(port) to enable; None disables the webhook server.
    pub webhook_port: Option<u16>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            socket_path: claw_pal::dirs::KernelDirs::socket_path().to_string_lossy().into_owned(),
            max_sessions: 100,
            provider_config: ProviderConfig::Dynamic,
            webhook_port: None,
        }
    }
}

/// The KernelServer that exposes agent functionality via JSON-RPC 2.0 over IPC.
// TODO(G-5): migrate webhook_server field from AxumWebhookServer to WebhookTriggerServer.
// Blocked on refactoring handle_trigger_add_webhook to use EventBus subscription
// instead of synchronous AgentLoop callbacks. See docs/gap-analysis.md §G-5.
#[allow(deprecated)]
pub struct KernelServer {
    /// Server configuration.
    config: ServerConfig,
    /// Session manager for active sessions.
    session_manager: Arc<SessionManager>,
    /// Shutdown signal.
    shutdown: Arc<RwLock<bool>>,
    /// Provider registry for multi-provider routing.
    registry: Arc<ProviderRegistry>,
    /// EventBus for broadcasting runtime events to IPC clients.
    pub event_bus: EventBus,
    /// Registry of connected channels.
    channel_registry: Arc<ChannelRegistry>,
    /// Multi-agent orchestrator.
    orchestrator: Arc<AgentOrchestrator>,
    /// IPC auth token (shared with all connections).
    pub auth_token: Arc<String>,
    /// Global server-level tool registry.
    tool_registry: Arc<GlobalToolRegistry>,
    /// Global server-level skill registry.
    skill_registry: Arc<GlobalSkillRegistry>,
    /// Built-in HTTP webhook server (Some if webhook_port was configured).
    webhook_server: Option<Arc<claw_runtime::webhook::AxumWebhookServer>>,
    /// Persistent trigger storage.
    trigger_store: Option<Arc<crate::trigger_store::TriggerStore>>,
    /// Active EventTrigger task handles, keyed by trigger_id.
    /// Used to abort the background listener when `trigger.remove` is called.
    event_trigger_handles: Arc<dashmap::DashMap<String, tokio::task::AbortHandle>>,
    /// Server-level scheduler (shared across all connections).
    scheduler: Arc<claw_runtime::TokioScheduler>,
    /// Shared channel router (for dynamic IPC-configurable routing rules).
    channel_router: Arc<claw_channel::router::ChannelRouter>,
    /// Audit log writer handle — cloned into each connection for external tool auditing.
    pub audit_log: claw_tools::audit::AuditLogWriterHandle,
    /// Shared in-memory audit ring buffer — queryable via `audit.list` IPC (G-16).
    pub audit_store: Arc<claw_tools::audit::AuditStore>,
    /// Background task that flushes audit events to disk (kept alive for server lifetime).
    _audit_log_task: tokio::task::JoinHandle<()>,
    /// Server-level hot-loader handle — exposes tool.watch_dir / tool.reload (G-11).
    hot_loader: crate::hot_loader::HotLoaderHandle,
    /// Background task driving the debounced hot-loader fan-out (kept alive for server lifetime).
    _hot_loader_task: tokio::task::JoinHandle<()>,
}

impl KernelServer {
    /// Creates a new KernelServer with the given configuration.
    ///
    /// # Example
    ///
    /// ```rust
    /// use claw_server::{KernelServer, ServerConfig, ProviderConfig};
    ///
    /// # fn example() {
    /// let config = ServerConfig {
    ///     socket_path: "/tmp/claw-kernel.sock".to_string(),
    ///     max_sessions: 100,
    ///     webhook_port: None,
    ///     provider_config: ProviderConfig::Anthropic {
    ///         api_key: "your-api-key".to_string(),
    ///         default_model: "claude-3-opus".to_string(),
    ///     },
    /// };
    ///
    /// let server = KernelServer::new(config);
    /// // server.run().await; // Run the server (requires async context)
    /// # }
    /// ```
    pub fn new(config: ServerConfig) -> Self {
        let session_manager = Arc::new(SessionManager::new(config.max_sessions));
        let provider = config.provider_config.build_provider();
        let provider_name = config.provider_config.provider_name().to_string();
        let registry = Arc::new(ProviderRegistry::new(provider_name, provider));

        // Generate a random auth token (32 random bytes, hex-encoded = 64 chars).
        let token = {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            use std::time::SystemTime;
            let mut hasher = DefaultHasher::new();
            SystemTime::now().hash(&mut hasher);
            std::process::id().hash(&mut hasher);
            format!("{:016x}{:016x}{:016x}{:016x}",
                hasher.finish(),
                {hasher.write_u64(0xdeadbeef); hasher.finish()},
                {hasher.write_u64(0xcafebabe); hasher.finish()},
                {hasher.write_u64(0x12345678); hasher.finish()},
            )
        };
        let event_bus = EventBus::new();
        let orchestrator = Arc::new(AgentOrchestrator::new(
            Arc::new(event_bus.clone()),
            Arc::new(claw_pal::TokioProcessManager::new()),
        ));
        orchestrator.start();

        // Build optional built-in webhook server.
        let webhook_server: Option<Arc<claw_runtime::webhook::AxumWebhookServer>> =
            if let Some(port) = config.webhook_port {
                use claw_runtime::webhook::{AxumWebhookServer, WebhookConfig};
                let wh_config = WebhookConfig::new("0.0.0.0", port);
                Some(Arc::new(AxumWebhookServer::new(wh_config)))
            } else {
                None
            };

        // Build trigger store (SQLite).
        let trigger_store = match claw_pal::dirs::KernelDirs::data_dir() {
            Ok(data_dir) => {
                let db_path = data_dir.join("triggers.db");
                match crate::trigger_store::TriggerStore::open(&db_path) {
                    Ok(store) => {
                        tracing::info!("Trigger store opened at {:?}", db_path);
                        Some(Arc::new(store))
                    }
                    Err(e) => {
                        tracing::warn!("Failed to open trigger store: {}; triggers will not persist", e);
                        None
                    }
                }
            }
            Err(_) => None,
        };

        let channel_router = Arc::new(claw_channel::router::ChannelRouterBuilder::new().build());

        // Start the server-wide audit log writer (one background task for the server lifetime).
        let (audit_log, audit_store, _audit_log_task) =
            claw_tools::audit::AuditLogWriter::start(claw_tools::audit::AuditLogConfig::default());

        // Initialise the server-level HotLoader (G-11): watches .lua/.js by default.
        let (hot_loader, _hot_loader_task) = crate::hot_loader::HotLoaderHandle::new(
            vec!["lua".to_string(), "js".to_string()],
        );

        Self {
            config,
            session_manager,
            shutdown: Arc::new(RwLock::new(false)),
            registry,
            event_bus,
            channel_registry: Arc::new(ChannelRegistry::new()),
            orchestrator,
            auth_token: Arc::new(token),
            tool_registry: Arc::new(GlobalToolRegistry::new()),
            skill_registry: Arc::new(GlobalSkillRegistry::new()),
            webhook_server,
            trigger_store,
            event_trigger_handles: Arc::new(dashmap::DashMap::new()),
            scheduler: Arc::new(claw_runtime::TokioScheduler::new()),
            channel_router,
            audit_log,
            audit_store,
            _audit_log_task,
            hot_loader,
            _hot_loader_task,
        }
    }

    /// Runs the server, listening for incoming connections.
    ///
    /// This method blocks until the server is shut down.
    pub async fn run(&self) -> Result<(), ServerError> {
        // Remove existing socket file if it exists
        if std::path::Path::new(&self.config.socket_path).exists() {
            std::fs::remove_file(&self.config.socket_path)
                .map_err(|_| ServerError::Ipc(claw_pal::error::IpcError::PermissionDenied))?;
        }

        // Start webhook server if configured
        if let Some(ref wh_server) = self.webhook_server {
            use claw_runtime::webhook::WebhookServer;
            if let Err(e) = wh_server.start().await {
                tracing::warn!("Failed to start webhook server: {}", e);
            } else {
                info!("Webhook server started on port {}", self.config.webhook_port.unwrap_or(0));
            }
        }

        // Restore persisted triggers
        if let Some(ref ts) = self.trigger_store {
            match ts.load_all() {
                Ok(triggers) => {
                    for t in triggers {
                        use claw_runtime::{Scheduler, TaskConfig, TaskTrigger};
                        match t.kind {
                            crate::trigger_store::TriggerKind::Cron => {
                                if let Some(ref cron_expr) = t.cron_expr {
                                    let orch = Arc::clone(&self.orchestrator);
                                    let agent = t.target_agent.clone();
                                    let msg = t.message.clone();
                                    let tid = t.trigger_id.clone();
                                    let config = TaskConfig::new(
                                        tid.clone(),
                                        TaskTrigger::Cron(cron_expr.clone()),
                                        move || {
                                            let orch = Arc::clone(&orch);
                                            let agent = agent.clone();
                                            let msg = msg.clone();
                                            let tid = tid.clone();
                                            Box::pin(async move {
                                                use claw_runtime::agent_types::AgentId;
                                                use claw_runtime::orchestrator::SteerCommand;
                                                let aid = AgentId::new(agent.clone());
                                                if let Err(e) = orch.steer(&aid, SteerCommand::Custom {
                                                    command: "inject".to_string(),
                                                    payload: msg,
                                                }).await {
                                                    tracing::warn!("Restored cron trigger {}: steer failed: {}", tid, e);
                                                }
                                            })
                                        },
                                    );
                                    if let Err(e) = self.scheduler.schedule(config).await {
                                        tracing::warn!("Failed to restore cron trigger {}: {}", t.trigger_id, e);
                                    }
                                }
                            }
                            crate::trigger_store::TriggerKind::Webhook => {
                                if let Some(ref wh_server) = self.webhook_server {
                                    use claw_runtime::webhook::{EndpointConfig, WebhookServer};
                                    let endpoint = t.endpoint.clone()
                                        .unwrap_or_else(|| format!("/hooks/{}", t.trigger_id));
                                    let target = t.target_agent.clone();
                                    let tid = t.trigger_id.clone();
                                    let sm = Arc::clone(&self.session_manager);
                                    let prov = Arc::clone(&self.registry).default_provider();
                                    let ch_reg = Arc::clone(&self.channel_registry);
                                    let eb = self.event_bus.clone();

                                    let ep_config = EndpointConfig::new(
                                        endpoint.clone(),
                                        move |req: claw_runtime::webhook::WebhookRequest| {
                                            let target = target.clone();
                                            let tid = tid.clone();
                                            let sm = Arc::clone(&sm);
                                            let prov = Arc::clone(&prov);
                                            let ch_reg = Arc::clone(&ch_reg);
                                            let eb = eb.clone();
                                            async move {
                                                let body = String::from_utf8_lossy(&req.body).to_string();

                                                // Publish TriggerEvent to EventBus.
                                                let payload = serde_json::from_str::<serde_json::Value>(&body)
                                                    .unwrap_or(serde_json::Value::Null);
                                                let trigger_event = claw_runtime::trigger_event::TriggerEvent::webhook(
                                                    tid.clone(),
                                                    payload,
                                                    Some(claw_runtime::agent_types::AgentId::new(target.clone())),
                                                );
                                                let _ = eb.publish(claw_runtime::events::Event::TriggerFired(trigger_event));

                                                // Route through the inbound session pipeline.
                                                let session = match crate::handler::get_or_create_inbound_session(
                                                    Some(&target),
                                                    &sm,
                                                    &prov,
                                                    &ch_reg,
                                                ).await {
                                                    Ok(s) => s,
                                                    Err(e) => {
                                                        tracing::warn!("restored webhook trigger {}: failed to get session: {}", tid, e);
                                                        return Ok::<_, claw_runtime::webhook::WebhookError>(
                                                            claw_runtime::webhook::WebhookResponse::error(500, e.to_string()),
                                                        );
                                                    }
                                                };

                                                let (chunk_tx, _chunk_rx) =
                                                    tokio::sync::mpsc::channel::<claw_loop::StreamChunk>(256);
                                                let mut loop_guard = session.agent_loop.lock().await;
                                                match loop_guard.run_streaming(body, chunk_tx).await {
                                                    Ok(result) => {
                                                        let resp_body = serde_json::json!({
                                                            "session_id": session.id,
                                                            "content": result.content,
                                                        });
                                                        Ok(claw_runtime::webhook::WebhookResponse::json(&resp_body)
                                                            .unwrap_or_else(|_| claw_runtime::webhook::WebhookResponse::ok()))
                                                    }
                                                    Err(e) => {
                                                        tracing::error!("restored webhook trigger {}: agent loop error: {}", tid, e);
                                                        Ok(claw_runtime::webhook::WebhookResponse::error(500, e.to_string()))
                                                    }
                                                }
                                            }
                                        },
                                    );

                                    match wh_server.register(ep_config).await {
                                        Ok(()) => tracing::info!(
                                            "Restored webhook trigger {} at {}",
                                            t.trigger_id, endpoint
                                        ),
                                        Err(e) => tracing::warn!(
                                            "Failed to restore webhook trigger {}: {}",
                                            t.trigger_id, e
                                        ),
                                    }
                                } else {
                                    tracing::debug!(
                                        "Webhook trigger {} skipped (no webhook_port configured)",
                                        t.trigger_id
                                    );
                                }
                            }
                            crate::trigger_store::TriggerKind::Event => {
                                if let Some(ref pattern) = t.event_pattern {
                                    let orch = Arc::clone(&self.orchestrator);
                                    let agent = t.target_agent.clone();
                                    let msg = t.message.clone();
                                    let tid = t.trigger_id.clone();
                                    let pattern_str = pattern.clone();
                                    let eh = Arc::clone(&self.event_trigger_handles);
                                    let mut rx = self.event_bus.subscribe();
                                    let task = tokio::spawn(async move {
                                        use claw_runtime::agent_types::AgentId;
                                        use claw_runtime::orchestrator::SteerCommand;
                                        let pattern_regex = match crate::event_trigger::build_event_pattern_regex(&pattern_str) {
                                            Ok(r) => r,
                                            Err(e) => {
                                                tracing::warn!("Failed to restore event trigger {}: {}", tid, e);
                                                return;
                                            }
                                        };
                                        loop {
                                            let event = match rx.recv().await {
                                                Ok(e) => e,
                                                Err(_) => break,
                                            };
                                            let name = crate::event_trigger::event_type_name(&event);
                                            if pattern_regex.is_match(name.as_ref()) {
                                                let payload = msg.clone().unwrap_or_default();
                                                let aid = AgentId::new(agent.clone());
                                                if let Err(e) = orch.steer(&aid, SteerCommand::Custom {
                                                    command: "inject".to_string(),
                                                    payload: Some(payload),
                                                }).await {
                                                    tracing::warn!("restored event trigger {}: steer failed: {}", tid, e);
                                                }
                                            }
                                        }
                                        eh.remove(&tid);
                                    });
                                    self.event_trigger_handles.insert(
                                        t.trigger_id.clone(),
                                        task.abort_handle(),
                                    );
                                    tracing::info!("Restored event trigger {} (pattern={})", t.trigger_id, pattern);
                                }
                            }
                        }
                    }
                }
                Err(e) => tracing::warn!("Failed to load persisted triggers: {}", e),
            }
        }

        // Auto-scan default skill directories (G-14 fix).
        // Priority: builtin (lowest) → ~/.claw/skills → ./skills (highest).
        // Only directories that exist are scanned; missing dirs are silently skipped.
        for dir in claw_skills::default_search_dirs() {
            if dir.exists() {
                match self.skill_registry.load_dir(dir.clone()).await {
                    Ok(n) => info!("Auto-loaded {} skill(s) from {:?}", n, dir),
                    Err(e) => warn!("Failed to auto-load skills from {:?}: {}", dir, e),
                }
            }
        }

        let listener = UnixListener::bind(&self.config.socket_path)
            .map_err(|_| ServerError::Ipc(claw_pal::error::IpcError::PermissionDenied))?;

        info!(
            "KernelServer listening on {} (provider: {}, max_sessions: {})",
            self.config.socket_path,
            self.config.provider_config.provider_name(),
            self.config.max_sessions
        );

        // Use the default provider from the registry, shared across connections.
        let provider: Arc<dyn LLMProvider> = Arc::clone(&self.registry).default_provider();
        let registry = Arc::clone(&self.registry);
        let event_bus = self.event_bus.clone();
        let channel_registry = Arc::clone(&self.channel_registry);
        let orchestrator = Arc::clone(&self.orchestrator);
        let auth_token = Arc::clone(&self.auth_token);
        let tool_registry = Arc::clone(&self.tool_registry);
        let skill_registry = Arc::clone(&self.skill_registry);
        let scheduler = Arc::clone(&self.scheduler);
        let channel_router = Arc::clone(&self.channel_router);
        let event_trigger_handles = Arc::clone(&self.event_trigger_handles);
        let audit_log = self.audit_log.clone();
        let audit_store = Arc::clone(&self.audit_store);
        let hot_loader = self.hot_loader.clone();

        loop {
            // Check for shutdown
            if *self.shutdown.read().await {
                info!("KernelServer shutting down gracefully");
                break;
            }

            // Accept new connection with timeout to allow shutdown checking
            match tokio::time::timeout(tokio::time::Duration::from_millis(100), listener.accept())
                .await
            {
                Ok(Ok((stream, _addr))) => {
                    debug!("New client connection accepted");
                    let session_manager = Arc::clone(&self.session_manager);
                    let provider_clone = Arc::clone(&provider);
                    let registry_clone = Arc::clone(&registry);
                    let event_bus_clone = event_bus.clone();
                    let channel_registry_clone = Arc::clone(&channel_registry);
                    let orchestrator_clone = Arc::clone(&orchestrator);
                    let auth_token_clone = Arc::clone(&auth_token);
                    let tool_registry_clone = Arc::clone(&tool_registry);
                    let skill_registry_clone = Arc::clone(&skill_registry);
                    let scheduler_clone = Arc::clone(&scheduler);
                    let channel_router_clone = Arc::clone(&channel_router);
                    let webhook_server_clone = self.webhook_server.as_ref().map(Arc::clone);
                    let trigger_store_clone = self.trigger_store.as_ref().map(Arc::clone);
                    let event_trigger_handles_clone = Arc::clone(&event_trigger_handles);
                    let audit_log_clone = audit_log.clone();
                    let audit_store_clone = Arc::clone(&audit_store);
                    let hot_loader_clone = hot_loader.clone();
                    tokio::spawn(async move {
                        if let Err(e) = crate::handler::handle_connection(
                            stream,
                            session_manager,
                            provider_clone,
                            registry_clone,
                            event_bus_clone,
                            channel_registry_clone,
                            orchestrator_clone,
                            auth_token_clone,
                            tool_registry_clone,
                            skill_registry_clone,
                            scheduler_clone,
                            webhook_server_clone,
                            trigger_store_clone,
                            event_trigger_handles_clone,
                            channel_router_clone,
                            audit_log_clone,
                            audit_store_clone,
                            hot_loader_clone,
                        )
                        .await
                        {
                            warn!("Connection handler error: {}", e);
                        }
                    });
                }
                Ok(Err(e)) => {
                    error!("Failed to accept connection: {}", e);
                }
                Err(_) => {
                    // Timeout - continue to check shutdown signal
                    continue;
                }
            }
        }

        // Cleanup
        if let Err(e) = std::fs::remove_file(&self.config.socket_path) {
            warn!("Failed to remove socket file: {}", e);
        }

        Ok(())
    }

    /// Initiates server shutdown.
    pub async fn shutdown(&self) {
        let mut shutdown = self.shutdown.write().await;
        *shutdown = true;
        info!("Shutdown signal sent");
    }

    /// Check if a kernel server is running at the given socket path.
    ///
    /// Attempts a non-blocking connection to the socket.
    /// Returns `true` if the socket exists and accepts connections.
    pub fn probe(socket_path: &str) -> bool {
        // Use std::os::unix::net to do a synchronous probe
        #[cfg(unix)]
        {
            std::os::unix::net::UnixStream::connect(socket_path).is_ok()
        }
        #[cfg(not(unix))]
        {
            let _ = socket_path;
            false
        }
    }

    /// Returns the session manager.
    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    /// Returns the server configuration.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Returns the channel registry.
    pub fn channel_registry(&self) -> &Arc<ChannelRegistry> {
        &self.channel_registry
    }

    /// Returns the agent orchestrator.
    pub fn orchestrator(&self) -> &Arc<AgentOrchestrator> {
        &self.orchestrator
    }

    /// Returns the global tool registry.
    pub fn tool_registry(&self) -> &Arc<GlobalToolRegistry> {
        &self.tool_registry
    }

    /// Returns the global skill registry.
    pub fn skill_registry(&self) -> &Arc<GlobalSkillRegistry> {
        &self.skill_registry
    }

    /// Returns the webhook server if configured.
    pub fn webhook_server(&self) -> Option<&Arc<claw_runtime::webhook::AxumWebhookServer>> {
        self.webhook_server.as_ref()
    }

    /// Returns the server-level scheduler.
    pub fn scheduler(&self) -> &Arc<claw_runtime::TokioScheduler> {
        &self.scheduler
    }

    /// Returns the channel router.
    pub fn channel_router(&self) -> &Arc<claw_channel::router::ChannelRouter> {
        &self.channel_router
    }

    /// Returns the server-level hot-loader handle.
    pub fn hot_loader(&self) -> &crate::hot_loader::HotLoaderHandle {
        &self.hot_loader
    }
}

impl std::fmt::Debug for KernelServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KernelServer")
            .field("config", &self.config)
            .field("session_manager", &self.session_manager)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_config_anthropic() {
        let config = ProviderConfig::Anthropic {
            api_key: "test-key".to_string(),
            default_model: "claude-3-opus".to_string(),
        };
        assert_eq!(config.provider_name(), "anthropic");
        assert_eq!(config.default_model(), Some("claude-3-opus"));
    }

    #[test]
    fn test_provider_config_openai() {
        let config = ProviderConfig::OpenAI {
            api_key: "test-key".to_string(),
            base_url: "https://api.openai.com".to_string(),
            default_model: "gpt-4".to_string(),
        };
        assert_eq!(config.provider_name(), "openai");
        assert_eq!(config.default_model(), Some("gpt-4"));
    }

    #[test]
    fn test_provider_config_ollama() {
        let config = ProviderConfig::Ollama {
            base_url: "http://localhost:11434".to_string(),
            default_model: "llama2".to_string(),
        };
        assert_eq!(config.provider_name(), "ollama");
        assert_eq!(config.default_model(), Some("llama2"));
    }

    #[test]
    fn test_provider_config_deepseek() {
        let config = ProviderConfig::DeepSeek {
            api_key: "test-key".to_string(),
            default_model: "deepseek-chat".to_string(),
        };
        assert_eq!(config.provider_name(), "deepseek");
    }

    #[test]
    fn test_provider_config_moonshot() {
        let config = ProviderConfig::Moonshot {
            api_key: "test-key".to_string(),
            default_model: "moonshot-v1".to_string(),
        };
        assert_eq!(config.provider_name(), "moonshot");
    }

    #[test]
    fn test_provider_config_dynamic() {
        let config = ProviderConfig::Dynamic;
        assert_eq!(config.provider_name(), "dynamic");
        assert_eq!(config.default_model(), None);
    }

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert!(!config.socket_path.is_empty());
        assert_eq!(config.max_sessions, 100);
        assert!(matches!(config.provider_config, ProviderConfig::Dynamic));
    }

    #[tokio::test]
    async fn test_kernel_server_new() {
        let config = ServerConfig::default();
        let server = KernelServer::new(config);
        assert_eq!(server.config().max_sessions, 100);
    }

    #[tokio::test]
    async fn test_kernel_server_shutdown() {
        let config = ServerConfig::default();
        let server = KernelServer::new(config);

        assert!(!*server.shutdown.read().await);
        server.shutdown().await;
        assert!(*server.shutdown.read().await);
    }
}
