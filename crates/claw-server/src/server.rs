//! KernelServer implementation.
//!
//! Provides a JSON-RPC 2.0 server over local IPC for remote agent control.

use std::sync::Arc;

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

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
            ProviderConfig::Dynamic => None,
        }
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
            socket_path: "/tmp/claw-kernel.sock".to_string(),
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
        Self {
            config,
            session_manager,
            shutdown: Arc::new(RwLock::new(false)),
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
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, session_manager).await {
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

    /// Returns the session manager.
    pub fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    /// Returns the server configuration.
    pub fn config(&self) -> &ServerConfig {
        &self.config
    }

    /// Handles a single client connection.
    async fn handle_connection(
        _stream: UnixStream,
        _session_manager: Arc<SessionManager>,
    ) -> Result<(), ServerError> {
        // TODO: Implement connection handling in handler.rs
        // This stub is here to make the server compile
        debug!("Handling new connection");
        Ok(())
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
        assert_eq!(config.socket_path, "/tmp/claw-kernel.sock");
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
