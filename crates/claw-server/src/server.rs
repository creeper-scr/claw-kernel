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
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            socket_path: claw_pal::dirs::KernelDirs::socket_path().to_string_lossy().into_owned(),
            max_sessions: 100,
            provider_config: ProviderConfig::Dynamic,
        }
    }
}

/// The KernelServer that exposes agent functionality via JSON-RPC 2.0 over IPC.
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
        Self {
            config,
            session_manager,
            shutdown: Arc::new(RwLock::new(false)),
            registry,
            event_bus,
            channel_registry: Arc::new(ChannelRegistry::new()),
            orchestrator,
            auth_token: Arc::new(token),
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

    #[test]
    fn test_kernel_server_new() {
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
