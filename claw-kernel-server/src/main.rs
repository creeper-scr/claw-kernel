use clap::Parser;
use claw_pal::dirs::KernelDirs;
use claw_server::{KernelServer, ProviderConfig, ServerConfig};

fn default_socket_path() -> String {
    KernelDirs::socket_path().to_string_lossy().into_owned()
}

#[derive(Parser, Debug)]
#[command(
    name = "claw-kernel-server",
    version,
    about = "claw-kernel IPC daemon — exposes agent loop via JSON-RPC 2.0"
)]
struct Cli {
    #[arg(long, env = "CLAW_SOCKET_PATH", default_value_t = default_socket_path())]
    socket_path: String,

    #[arg(long, env = "CLAW_PROVIDER", default_value = "anthropic")]
    provider: String,

    #[arg(long, env = "CLAW_MODEL")]
    model: Option<String>,

    #[arg(long, env = "CLAW_API_KEY")]
    api_key: Option<String>,

    #[arg(long, env = "CLAW_BASE_URL")]
    base_url: Option<String>,

    #[arg(long, env = "CLAW_MAX_SESSIONS", default_value = "16")]
    max_sessions: usize,

    #[arg(long, env = "CLAW_POWER_KEY")]
    power_key: Option<String>,

    #[arg(long, env = "RUST_LOG", default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(&cli.log_level)
        .init();

    // Acquire daemon lock to prevent duplicate instances
    let pid_path = KernelDirs::pid_path();
    let pid = std::process::id();

    match claw_pal::lockfile::DaemonLock::acquire(&pid_path, pid) {
        Ok(_lock) => {
            // Lock acquired — we are the only daemon, proceed
            tracing::info!("Acquired daemon lock at {}", pid_path.display());

            let provider_config = build_provider_config(&cli)?;
            let socket_path = cli.socket_path.clone();
            let config = ServerConfig {
                socket_path: cli.socket_path,
                max_sessions: cli.max_sessions,
                webhook_port: None,
                provider_config,
            };

            let server = KernelServer::new(config);

            // Write auth token to file (mode 0o600) for SDK auto-discovery.
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let token_path = KernelDirs::data_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
                    .join("kernel.token");
                if let Some(parent) = token_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&token_path)
                {
                    Ok(mut f) => {
                        let _ = f.write_all(server.auth_token.as_bytes());
                        tracing::info!("Auth token written to {}", token_path.display());
                    }
                    Err(e) => tracing::warn!("Failed to write auth token: {}", e),
                }
            }

            tracing::info!("claw-kernel-server starting on {}", &socket_path);
            server.run().await?;
        }
        Err(claw_pal::lockfile::LockError::AlreadyRunning { pid: existing_pid }) => {
            tracing::info!(
                "claw-kernel-server already running (PID {}). Exiting.",
                existing_pid
            );
            // Exit cleanly — this is not an error
            std::process::exit(0);
        }
        Err(e) => {
            tracing::warn!("Could not acquire daemon lock: {}. Proceeding without lock.", e);
            // If lock fails for I/O reasons, proceed without it
            let provider_config = build_provider_config(&cli)?;
            let socket_path = cli.socket_path.clone();
            let config = ServerConfig {
                socket_path: cli.socket_path,
                max_sessions: cli.max_sessions,
                webhook_port: None,
                provider_config,
            };
            let server = KernelServer::new(config);

            // Write auth token to file (mode 0o600) for SDK auto-discovery.
            {
                use std::io::Write;
                use std::os::unix::fs::OpenOptionsExt;
                let token_path = KernelDirs::data_dir()
                    .unwrap_or_else(|_| std::path::PathBuf::from("/tmp"))
                    .join("kernel.token");
                if let Some(parent) = token_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .mode(0o600)
                    .open(&token_path)
                {
                    Ok(mut f) => {
                        let _ = f.write_all(server.auth_token.as_bytes());
                        tracing::info!("Auth token written to {}", token_path.display());
                    }
                    Err(e) => tracing::warn!("Failed to write auth token: {}", e),
                }
            }

            tracing::info!("claw-kernel-server starting on {}", &socket_path);
            server.run().await?;
        }
    }

    Ok(())
}

fn build_provider_config(cli: &Cli) -> anyhow::Result<ProviderConfig> {
    match cli.provider.as_str() {
        "anthropic" => Ok(ProviderConfig::Anthropic {
            api_key: cli.api_key.clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .unwrap_or_default(),
            default_model: cli.model.clone()
                .unwrap_or_else(|| "claude-sonnet-4-6".to_string()),
        }),
        "openai" => Ok(ProviderConfig::OpenAI {
            api_key: cli.api_key.clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .unwrap_or_default(),
            base_url: cli.base_url.clone()
                .unwrap_or_else(|| "https://api.openai.com".to_string()),
            default_model: cli.model.clone().unwrap_or_else(|| "gpt-4o".to_string()),
        }),
        "ollama" => Ok(ProviderConfig::Ollama {
            base_url: cli.base_url.clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
            default_model: cli.model.clone().unwrap_or_else(|| "llama3".to_string()),
        }),
        "deepseek" => Ok(ProviderConfig::DeepSeek {
            api_key: cli.api_key.clone()
                .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
                .unwrap_or_default(),
            default_model: cli.model.clone()
                .unwrap_or_else(|| "deepseek-chat".to_string()),
        }),
        "moonshot" => Ok(ProviderConfig::Moonshot {
            api_key: cli.api_key.clone()
                .or_else(|| std::env::var("MOONSHOT_API_KEY").ok())
                .unwrap_or_default(),
            default_model: cli.model.clone()
                .unwrap_or_else(|| "moonshot-v1-8k".to_string()),
        }),
        "gemini" => Ok(ProviderConfig::Gemini {
            api_key: cli.api_key.clone()
                .or_else(|| std::env::var("GEMINI_API_KEY").ok())
                .unwrap_or_default(),
            default_model: cli.model.clone()
                .unwrap_or_else(|| "gemini-2.0-flash".to_string()),
        }),
        "mistral" => Ok(ProviderConfig::Mistral {
            api_key: cli.api_key.clone()
                .or_else(|| std::env::var("MISTRAL_API_KEY").ok())
                .unwrap_or_default(),
            default_model: cli.model.clone()
                .unwrap_or_else(|| "mistral-large-latest".to_string()),
        }),
        "azure-openai" => Ok(ProviderConfig::AzureOpenAI {
            api_key: cli.api_key.clone()
                .or_else(|| std::env::var("AZURE_OPENAI_API_KEY").ok())
                .unwrap_or_default(),
            resource_name: std::env::var("AZURE_OPENAI_RESOURCE").unwrap_or_default(),
            deployment_id: cli.model.clone()
                .or_else(|| std::env::var("AZURE_OPENAI_DEPLOYMENT").ok())
                .unwrap_or_default(),
            api_version: std::env::var("AZURE_OPENAI_API_VERSION")
                .unwrap_or_else(|_| "2024-02-01".to_string()),
        }),
        other => anyhow::bail!("unknown provider: {}", other),
    }
}
