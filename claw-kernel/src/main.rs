//! claw-kernel 主程序入口
//!
//! 实现 CLI 解析、配置加载、目录初始化和主事件循环

use anyhow::{Context, Result};
use clap::Parser;
use std::process;
use tracing::{error, info, warn};
use zeroize::Zeroizing;

mod cli;

use cli::{Cli, Commands, ExecutionMode, LogLevel};

/// Power Key environment variable name
const POWER_KEY_ENV_VAR: &str = "CLAW_KERNEL_POWER_KEY";

#[tokio::main]
async fn main() {
    // 解析命令行参数
    let cli = Cli::parse();

    // 初始化日志系统
    init_logging(cli.log_level);

    // 执行命令或启动内核
    if let Err(e) = run(cli).await {
        error!("Error: {}", e);
        process::exit(1);
    }
}

/// 初始化日志系统
fn init_logging(level: LogLevel) {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::from(level))
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .init();
}

/// 主运行逻辑
async fn run(cli: Cli) -> Result<()> {
    // 处理子命令
    match cli.command {
        Some(Commands::Init { force }) => {
            info!("初始化 claw-kernel 配置目录...");
            claw_pal::config::init_directories(force).context("初始化目录失败")?;
            info!("初始化完成！");
            Ok(())
        }

        Some(Commands::Dirs) => {
            print_directories();
            Ok(())
        }

        Some(Commands::Version) => {
            print_version();
            Ok(())
        }

        Some(Commands::SetPowerKey { key }) => {
            info!("设置 Power Key...");

            // Save the power key using PowerKeyManager
            claw_pal::security::PowerKeyManager::save_power_key(&key)
                .map_err(|e| anyhow::anyhow!("Power Key 保存失败: {}", e))?;

            info!("Power Key 设置成功！");
            Ok(())
        }

        Some(Commands::VerifyPowerKey { key }) => {
            match claw_pal::security::PowerKeyValidator::validate(&key) {
                Ok(()) => {
                    println!("Power Key 格式有效");
                    Ok(())
                }
                Err(e) => Err(anyhow::anyhow!("Power Key 无效: {}", e)),
            }
        }

        Some(Commands::Config { default }) => {
            if default {
                let config = claw_pal::config::KernelConfig::default();
                println!("{}", toml::to_string_pretty(&config)?);
            } else {
                // 加载并显示当前配置
                let config = load_config(cli.config.as_deref()).await?;
                println!("{}", toml::to_string_pretty(&config)?);
            }
            Ok(())
        }

        Some(Commands::Run {
            agent_id,
            tools_dir,
            once,
        }) => {
            info!("启动 claw-kernel...");
            info!("执行模式: {}", cli.mode);

            if cli.mode == ExecutionMode::Power {
                // Resolve power key from CLI > Env > Config file
                let power_key = resolve_power_key(cli.power_key)?;

                info!("验证 Power Key...");

                // Load stored hash and verify
                let stored_hash = claw_pal::security::PowerKeyManager::load_stored_hash()
                    .map_err(|e| anyhow::anyhow!("无法加载已保存的 Power Key，请先运行 'claw-kernel set-power-key <key>': {}", e))?;

                match claw_pal::security::ModeTransitionGuard::enter_power_mode(
                    &power_key,
                    &stored_hash,
                ) {
                    Ok(_) => info!("Power Key 验证通过，已进入 Power Mode"),
                    Err(e) => return Err(anyhow::anyhow!("Power Key 验证失败: {}", e)),
                }
            }

            // 初始化目录（如果不存在）
            claw_pal::config::init_directories(false).context("初始化目录失败")?;

            // 加载配置
            let _config = load_config(cli.config.as_deref()).await?;
            info!("配置加载完成");

            // TODO: 启动 Agent 运行时
            info!("Agent ID: {:?}", agent_id);
            info!("Tools Dir: {:?}", tools_dir);
            info!("单次模式: {}", once);

            warn!("Agent 运行时实现即将到来...");
            Ok(())
        }

        None => {
            // 没有子命令，启动默认模式
            info!("启动 claw-kernel (Safe Mode)...");

            // 初始化目录（如果不存在）
            claw_pal::config::init_directories(false).context("初始化目录失败")?;

            // 加载配置
            let _config = load_config(cli.config.as_deref()).await?;
            info!("配置加载完成");

            // TODO: 启动默认服务
            info!("claw-kernel 已启动");
            Ok(())
        }
    }
}

/// 加载配置
async fn load_config(
    config_path: Option<&std::path::Path>,
) -> Result<claw_pal::config::KernelConfig> {
    use claw_pal::config::ConfigLoader;

    let loader = ConfigLoader::new();

    if let Some(path) = config_path {
        Ok(loader.load_from_file(path)?)
    } else {
        Ok(loader.load()?)
    }
}

/// Resolve the effective Power Key following priority order:
/// 1. CLI argument (`--power-key`)
/// 2. Environment variable (`CLAW_KERNEL_POWER_KEY`)
/// 3. Config file (`~/.config/claw-kernel/power.key`)
///
/// Returns the plaintext key for verification, or an error if no key is found.
///
/// The returned `Zeroizing<String>` automatically zeroes the plaintext key
/// when it is dropped, preventing the secret from lingering in freed memory.
fn resolve_power_key(cli_key: Option<String>) -> Result<Zeroizing<String>> {
    // Priority 1: CLI argument
    if let Some(key) = cli_key {
        if !key.is_empty() {
            return Ok(Zeroizing::new(key));
        }
    }

    // Priority 2: Environment variable
    if let Ok(env_key) = std::env::var(POWER_KEY_ENV_VAR) {
        if !env_key.is_empty() {
            return Ok(Zeroizing::new(env_key));
        }
    }

    // Priority 3: Config file exists - user needs to provide key via CLI or env
    if claw_pal::security::PowerKeyManager::is_configured() {
        return Err(anyhow::anyhow!(
            "Power Key 已配置在 {}。请通过 --power-key 参数或 {} 环境变量提供密钥",
            claw_pal::dirs::power_key_path()
                .unwrap_or_default()
                .display(),
            POWER_KEY_ENV_VAR
        ));
    }

    Err(anyhow::anyhow!(
        "未配置 Power Key。请先运行 'claw-kernel set-power-key <key>' 进行设置"
    ))
}

/// 打印目录信息
fn print_directories() {
    println!("claw-kernel 目录结构:");
    println!();

    if let Some(dir) = claw_pal::dirs::config_dir() {
        println!("  配置目录: {}", dir.display());
    }
    if let Some(dir) = claw_pal::dirs::data_dir() {
        println!("  数据目录: {}", dir.display());
    }
    if let Some(dir) = claw_pal::dirs::cache_dir() {
        println!("  缓存目录: {}", dir.display());
    }
    if let Some(dir) = claw_pal::dirs::tools_dir() {
        println!("  工具目录: {}", dir.display());
    }
    if let Some(dir) = claw_pal::dirs::scripts_dir() {
        println!("  脚本目录: {}", dir.display());
    }
    if let Some(dir) = claw_pal::dirs::logs_dir() {
        println!("  日志目录: {}", dir.display());
    }
    if let Some(dir) = claw_pal::dirs::agents_dir() {
        println!("  Agent目录: {}", dir.display());
    }
}

/// 打印版本信息
fn print_version() {
    println!("claw-kernel {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("构建信息:");
    println!("  目标平台: {}", std::env::consts::ARCH);
    println!("  操作系统: {}", std::env::consts::OS);
    println!("  Rust版本: {}", rustc_version_runtime::version());
    println!();
    println!("已启用功能:");
    println!("  - Lua脚本引擎");
    println!("  - 热加载工具");
    println!("  - SQLite内存存储");
}
