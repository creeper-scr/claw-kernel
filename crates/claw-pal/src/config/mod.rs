//! Configuration management for claw-kernel.
//!
//! Provides layered configuration: defaults < TOML file < environment variables < CLI args

use figment::{
    providers::{Env, Format, Toml},
    Figment, Profile,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Main kernel configuration.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct KernelConfig {
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
}

/// LLM provider configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProviderConfig {
    #[serde(default = "default_provider")]
    pub name: String,
    pub api_key: Option<String>,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

/// Sandbox/execution mode configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SandboxConfig {
    #[serde(default = "default_mode")]
    pub mode: ExecutionMode,
    #[serde(default)]
    pub filesystem: Vec<FileSystemRule>,
    #[serde(default)]
    pub network: NetworkConfig,
}

/// Execution mode for sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionMode {
    #[default]
    Safe,
    Power,
}

/// Filesystem access rule.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FileSystemRule {
    pub path: String,
    pub access: FileAccess,
}

/// File access level.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FileAccess {
    ReadOnly,
    ReadWrite,
    Deny,
}

/// Network configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetworkConfig {
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub allowed_ports: Vec<u16>,
    #[serde(default = "default_allow_loopback")]
    pub allow_loopback: bool,
}

/// Logging configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_audit_log")]
    pub audit_log: bool,
    #[serde(default = "default_audit_retention_days")]
    pub audit_retention_days: u32,
    #[serde(default = "default_log_dir")]
    pub log_dir: PathBuf,
}

/// Tools configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ToolsConfig {
    #[serde(default = "default_hot_reload")]
    pub hot_reload: bool,
    #[serde(default)]
    pub watch_dirs: Vec<PathBuf>,
    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,
    #[serde(default = "default_compile_timeout_secs")]
    pub compile_timeout_secs: u64,
    #[serde(default = "default_keep_previous_secs")]
    pub keep_previous_secs: u64,
}

/// Runtime configuration.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuntimeConfig {
    #[serde(default = "default_max_agents")]
    pub max_agents: usize,
    #[serde(default = "default_ipc_socket_path")]
    pub ipc_socket_path: PathBuf,
}

// Default value functions

fn default_provider() -> String {
    "anthropic".to_string()
}
fn default_model() -> String {
    "claude-3-sonnet-20240229".to_string()
}
fn default_timeout_secs() -> u64 {
    60
}
fn default_max_retries() -> u32 {
    3
}
fn default_mode() -> ExecutionMode {
    ExecutionMode::Safe
}
fn default_allow_loopback() -> bool {
    true
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_audit_log() -> bool {
    true
}
fn default_audit_retention_days() -> u32 {
    30
}
fn default_log_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("claw-kernel/logs")
}
fn default_hot_reload() -> bool {
    false
}
fn default_debounce_ms() -> u64 {
    50
}
fn default_compile_timeout_secs() -> u64 {
    30
}
fn default_keep_previous_secs() -> u64 {
    60
}
fn default_max_agents() -> usize {
    16
}
fn default_ipc_socket_path() -> PathBuf {
    dirs::runtime_dir()
        .or_else(dirs::cache_dir)
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("claw-kernel/ipc.sock")
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            name: default_provider(),
            api_key: None,
            model: default_model(),
            base_url: None,
            timeout_secs: default_timeout_secs(),
            max_retries: default_max_retries(),
        }
    }
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: default_mode(),
            filesystem: vec![],
            network: NetworkConfig::default(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            allowed_domains: vec![],
            allowed_ports: vec![],
            allow_loopback: default_allow_loopback(),
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            audit_log: default_audit_log(),
            audit_retention_days: default_audit_retention_days(),
            log_dir: default_log_dir(),
        }
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            hot_reload: default_hot_reload(),
            watch_dirs: vec![],
            debounce_ms: default_debounce_ms(),
            compile_timeout_secs: default_compile_timeout_secs(),
            keep_previous_secs: default_keep_previous_secs(),
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_agents: default_max_agents(),
            ipc_socket_path: default_ipc_socket_path(),
        }
    }
}

/// Configuration error types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    FileNotFound(String),
    ParseError(String),
    InvalidValue(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::FileNotFound(p) => write!(f, "Configuration file not found: {}", p),
            ConfigError::ParseError(e) => write!(f, "Failed to parse configuration: {}", e),
            ConfigError::InvalidValue(e) => write!(f, "Invalid configuration value: {}", e),
        }
    }
}

impl std::error::Error for ConfigError {}

impl KernelConfig {
    /// Load configuration from file with environment variable overrides.
    pub fn load(config_path: &str) -> Result<Self, ConfigError> {
        let expanded = shellexpand::tilde(config_path);
        let config_file = expanded.to_string();

        // If file doesn't exist, use defaults
        if !std::path::Path::new(&config_file).exists() {
            return Ok(Self::default());
        }

        let figment = Figment::new()
            .merge(Toml::file(&config_file).nested())
            .merge(Env::prefixed("CLAW_KERNEL_").split("__"))
            .select(Profile::Default);

        figment
            .extract()
            .map_err(|e| ConfigError::ParseError(e.to_string()))
    }

    /// Get the configuration directory path.
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("claw-kernel")
    }

    /// Get the data directory path.
    pub fn data_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("claw-kernel")
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), ConfigError> {
        // Validate timeout
        if self.provider.timeout_secs == 0 {
            return Err(ConfigError::InvalidValue(
                "provider.timeout_secs must be greater than 0".to_string(),
            ));
        }

        // Validate max_agents
        if self.runtime.max_agents == 0 {
            return Err(ConfigError::InvalidValue(
                "runtime.max_agents must be greater than 0".to_string(),
            ));
        }

        Ok(())
    }
}

/// Configuration loader for convenience.
pub struct ConfigLoader;

impl ConfigLoader {
    /// Create a new ConfigLoader.
    pub fn new() -> Self {
        Self
    }

    /// Load configuration from default locations.
    pub fn load(&self) -> Result<KernelConfig, ConfigError> {
        let config_path = KernelConfig::config_dir().join("config.toml");
        KernelConfig::load(
            config_path
                .to_str()
                .unwrap_or("~/.config/claw-kernel/config.toml"),
        )
    }

    /// Load configuration from a specific file.
    pub fn load_from_file(&self, path: &std::path::Path) -> Result<KernelConfig, ConfigError> {
        KernelConfig::load(path.to_str().unwrap_or(""))
    }
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self::new()
    }
}

/// Initialize all configuration directories.
pub fn init_directories(_force: bool) -> Result<(), std::io::Error> {
    use crate::dirs;

    // Create config directory
    if let Some(dir) = dirs::config_dir() {
        std::fs::create_dir_all(&dir)?;
    }

    // Create data directories
    if let Some(dir) = dirs::data_dir() {
        std::fs::create_dir_all(&dir)?;
    }
    if let Some(dir) = dirs::logs_dir() {
        std::fs::create_dir_all(&dir)?;
    }
    if let Some(dir) = dirs::agents_dir() {
        std::fs::create_dir_all(&dir)?;
    }
    if let Some(dir) = dirs::tools_dir() {
        std::fs::create_dir_all(&dir)?;
    }
    if let Some(dir) = dirs::scripts_dir() {
        std::fs::create_dir_all(&dir)?;
    }
    if let Some(dir) = dirs::cache_dir() {
        std::fs::create_dir_all(&dir)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = KernelConfig::default();
        assert_eq!(config.provider.name, "anthropic");
        assert_eq!(config.sandbox.mode, ExecutionMode::Safe);
        assert_eq!(config.logging.level, "info");
        assert_eq!(config.tools.debounce_ms, 50);
        assert_eq!(config.runtime.max_agents, 16);
    }

    #[test]
    fn test_config_validation() {
        let mut config = KernelConfig::default();
        assert!(config.validate().is_ok());

        config.provider.timeout_secs = 0;
        assert!(config.validate().is_err());
    }
}
