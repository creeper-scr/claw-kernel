use clap::Parser;
use claw_server::{KernelServer, ProviderConfig, ServerConfig};

#[derive(Parser, Debug)]
#[command(
    name = "claw-kernel-server",
    version,
    about = "claw-kernel IPC daemon — exposes agent loop via JSON-RPC 2.0"
)]
struct Cli {
    #[arg(
        long,
        env = "CLAW_SOCKET_PATH",
        default_value = "/tmp/claw-kernel.sock"
    )]
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

    let provider_config = build_provider_config(&cli)?;

    let socket_path = cli.socket_path.clone();
    let config = ServerConfig {
        socket_path: cli.socket_path,
        max_sessions: cli.max_sessions,
        provider_config,
    };

    let server = KernelServer::new(config);

    tracing::info!("claw-kernel-server starting on {}", &socket_path);
    server.run().await?;
    Ok(())
}

fn build_provider_config(cli: &Cli) -> anyhow::Result<ProviderConfig> {
    match cli.provider.as_str() {
        "anthropic" => Ok(ProviderConfig::Anthropic {
            api_key: cli
                .api_key
                .clone()
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .unwrap_or_default(),
            default_model: cli
                .model
                .clone()
                .unwrap_or_else(|| "claude-sonnet-4-6".to_string()),
        }),
        "openai" => Ok(ProviderConfig::OpenAI {
            api_key: cli
                .api_key
                .clone()
                .or_else(|| std::env::var("OPENAI_API_KEY").ok())
                .unwrap_or_default(),
            base_url: cli
                .base_url
                .clone()
                .unwrap_or_else(|| "https://api.openai.com".to_string()),
            default_model: cli.model.clone().unwrap_or_else(|| "gpt-4o".to_string()),
        }),
        "ollama" => Ok(ProviderConfig::Ollama {
            base_url: cli
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434".to_string()),
            default_model: cli.model.clone().unwrap_or_else(|| "llama3".to_string()),
        }),
        "deepseek" => Ok(ProviderConfig::DeepSeek {
            api_key: cli
                .api_key
                .clone()
                .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
                .unwrap_or_default(),
            default_model: cli
                .model
                .clone()
                .unwrap_or_else(|| "deepseek-chat".to_string()),
        }),
        "moonshot" => Ok(ProviderConfig::Moonshot {
            api_key: cli
                .api_key
                .clone()
                .or_else(|| std::env::var("MOONSHOT_API_KEY").ok())
                .unwrap_or_default(),
            default_model: cli
                .model
                .clone()
                .unwrap_or_else(|| "moonshot-v1-8k".to_string()),
        }),
        other => anyhow::bail!("unknown provider: {}", other),
    }
}
