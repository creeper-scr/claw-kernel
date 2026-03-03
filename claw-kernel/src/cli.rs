//! CLI 参数解析模块
//!
//! 使用 clap derive 宏定义完整的命令行接口
//! 实现 Safe Mode / Power Mode 切换、配置管理、版本信息等

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// claw-kernel: AI Agent 内核系统
///
/// 跨平台的 Agent 内核，支持 Lua 脚本扩展和热加载
#[derive(Debug, Parser)]
#[command(
    name = "claw-kernel",
    version = env!("CARGO_PKG_VERSION"),
    about = "AI Agent Kernel System",
    long_about = "claw-kernel 是 Claw 生态系统的共享基础设施库，\n\
                  提供 LLM 提供程序 HTTP 调用、工具使用协议、Agent 循环和内存系统。"
)]
pub struct Cli {
    /// 配置文件路径
    #[arg(
        short,
        long,
        value_name = "PATH",
        help = "配置文件路径 (默认: ~/.config/claw-kernel/config.toml)"
    )]
    pub config: Option<PathBuf>,

    /// 执行模式
    #[arg(
        short,
        long,
        value_enum,
        help = "执行模式: safe (默认) 或 power",
        default_value = "safe"
    )]
    pub mode: ExecutionMode,

    /// Power Key (用于进入 Power Mode)
    #[arg(
        long,
        value_name = "KEY",
        help = "Power Mode 密钥 (可通过环境变量 CLAW_KERNEL_POWER_KEY 设置)",
        env = "CLAW_KERNEL_POWER_KEY"
    )]
    pub power_key: Option<String>,

    /// 日志级别
    #[arg(short, long, value_enum, help = "日志级别", default_value = "info")]
    pub log_level: LogLevel,

    /// 子命令
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// 执行模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ExecutionMode {
    /// Safe Mode: 沙箱限制，默认模式
    Safe,
    /// Power Mode: 完整系统访问，需要 Power Key
    Power,
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionMode::Safe => write!(f, "safe"),
            ExecutionMode::Power => write!(f, "power"),
        }
    }
}

/// 日志级别
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::Trace => write!(f, "trace"),
            LogLevel::Debug => write!(f, "debug"),
            LogLevel::Info => write!(f, "info"),
            LogLevel::Warn => write!(f, "warn"),
            LogLevel::Error => write!(f, "error"),
        }
    }
}

impl From<LogLevel> for tracing::Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => tracing::Level::TRACE,
            LogLevel::Debug => tracing::Level::DEBUG,
            LogLevel::Info => tracing::Level::INFO,
            LogLevel::Warn => tracing::Level::WARN,
            LogLevel::Error => tracing::Level::ERROR,
        }
    }
}

/// 子命令
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// 初始化配置目录
    #[command(name = "init", about = "初始化配置目录和默认配置文件")]
    Init {
        /// 强制覆盖现有配置
        #[arg(short, long, help = "强制覆盖现有配置")]
        force: bool,
    },

    /// 设置 Power Key
    #[command(name = "set-power-key", about = "设置或更新 Power Mode 密钥")]
    SetPowerKey {
        /// 新的 Power Key
        #[arg(help = "新的 Power Key (至少12字符，包含至少2种字符类型)")]
        key: String,
    },

    /// 验证 Power Key
    #[command(name = "verify-power-key", about = "验证 Power Key 是否有效")]
    VerifyPowerKey {
        /// 要验证的 Power Key
        #[arg(help = "要验证的 Power Key")]
        key: String,
    },

    /// 显示配置信息
    #[command(name = "config", about = "显示当前配置")]
    Config {
        /// 显示默认配置而不是当前配置
        #[arg(short, long, help = "显示默认配置")]
        default: bool,
    },

    /// 运行 Agent
    #[command(name = "run", about = "运行 Agent")]
    Run {
        /// Agent ID
        #[arg(short, long, help = "Agent 唯一标识")]
        agent_id: Option<String>,

        /// 工具目录
        #[arg(short, long, value_name = "PATH", help = "工具脚本目录")]
        tools_dir: Option<PathBuf>,

        /// 单次模式 (运行一次后退出)
        #[arg(short, long, help = "单次模式")]
        once: bool,
    },

    /// 显示目录信息
    #[command(name = "dirs", about = "显示 claw-kernel 使用的目录路径")]
    Dirs,

    /// 版本信息
    #[command(name = "version", about = "显示版本信息")]
    Version,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_parser() {
        // 验证 CLI 定义正确
        Cli::command().debug_assert();
    }

    #[test]
    fn test_execution_mode_display() {
        assert_eq!(ExecutionMode::Safe.to_string(), "safe");
        assert_eq!(ExecutionMode::Power.to_string(), "power");
    }

    #[test]
    fn test_log_level_display() {
        assert_eq!(LogLevel::Trace.to_string(), "trace");
        assert_eq!(LogLevel::Debug.to_string(), "debug");
        assert_eq!(LogLevel::Info.to_string(), "info");
        assert_eq!(LogLevel::Warn.to_string(), "warn");
        assert_eq!(LogLevel::Error.to_string(), "error");
    }

    #[test]
    fn test_log_level_into_tracing() {
        assert_eq!(tracing::Level::from(LogLevel::Trace), tracing::Level::TRACE);
        assert_eq!(tracing::Level::from(LogLevel::Debug), tracing::Level::DEBUG);
        assert_eq!(tracing::Level::from(LogLevel::Info), tracing::Level::INFO);
        assert_eq!(tracing::Level::from(LogLevel::Warn), tracing::Level::WARN);
        assert_eq!(tracing::Level::from(LogLevel::Error), tracing::Level::ERROR);
    }

    #[test]
    fn test_cli_parse_default() {
        let cli = Cli::parse_from(["claw-kernel"]);
        assert!(cli.config.is_none());
        assert!(matches!(cli.mode, ExecutionMode::Safe));
        assert!(cli.power_key.is_none());
        assert!(matches!(cli.log_level, LogLevel::Info));
        assert!(cli.command.is_none());
    }

    #[test]
    fn test_cli_parse_with_config() {
        let cli = Cli::parse_from(["claw-kernel", "-c", "/tmp/test.toml"]);
        assert_eq!(cli.config, Some(PathBuf::from("/tmp/test.toml")));
    }

    #[test]
    fn test_cli_parse_with_mode() {
        let cli = Cli::parse_from(["claw-kernel", "--mode", "power"]);
        assert!(matches!(cli.mode, ExecutionMode::Power));
    }

    #[test]
    fn test_cli_parse_init_command() {
        let cli = Cli::parse_from(["claw-kernel", "init"]);
        assert!(
            matches!(cli.command, Some(Commands::Init { force: false })),
            "init command without --force should have force=false"
        );
    }

    #[test]
    fn test_cli_parse_init_command_with_force() {
        let cli = Cli::parse_from(["claw-kernel", "init", "--force"]);
        assert!(
            matches!(cli.command, Some(Commands::Init { force: true })),
            "init command with --force should have force=true"
        );
    }

    #[test]
    fn test_cli_parse_dirs_command() {
        let cli = Cli::parse_from(["claw-kernel", "dirs"]);
        assert!(matches!(cli.command, Some(Commands::Dirs)));
    }

    #[test]
    fn test_cli_parse_version_command() {
        let cli = Cli::parse_from(["claw-kernel", "version"]);
        assert!(matches!(cli.command, Some(Commands::Version)));
    }

    #[test]
    fn test_cli_parse_run_command() {
        let cli = Cli::parse_from(["claw-kernel", "run", "--agent-id", "test123", "--once"]);
        assert!(
            matches!(cli.command, Some(Commands::Run { agent_id: Some(ref id), once: true, .. }) if id == "test123")
        );
    }

    #[test]
    fn test_cli_env_var_name() {
        // Verify that the environment variable is CLAW_KERNEL_POWER_KEY
        // This is checked by looking at the help text
        let cmd = Cli::command();
        let power_key_arg = cmd.get_arguments().find(|a| a.get_id() == "power_key");
        assert!(power_key_arg.is_some());
        let arg = power_key_arg.unwrap();
        assert!(arg.get_env().is_some());
        assert_eq!(
            arg.get_env().unwrap().to_str(),
            Some("CLAW_KERNEL_POWER_KEY")
        );
    }
}
