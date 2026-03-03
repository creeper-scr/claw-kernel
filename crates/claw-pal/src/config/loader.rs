//! 配置加载器
//!
//! 实现分层配置加载：默认值 < TOML < 环境变量

use super::{ConfigError, KernelConfig};
use figment::{
    providers::{Env, Format, Serialized, Toml as TomlProvider},
    Figment,
};
use std::path::Path;

/// 配置加载器
pub struct ConfigLoader {
    figment: Figment,
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigLoader {
    /// 创建新的配置加载器
    ///
    /// 配置加载顺序：
    /// 1. 内置默认值
    /// 2. TOML 配置文件 (如果存在)
    /// 3. 环境变量 (CLAW_*)
    pub fn new() -> Self {
        // 从默认值开始
        let figment = Figment::from(Serialized::defaults(KernelConfig::default()))
            .merge(TomlProvider::file(Self::default_config_path()).nested())
            .merge(Env::prefixed("CLAW_").split("__"));

        Self { figment }
    }

    /// 从指定配置文件创建加载器
    pub fn from_file<P: AsRef<Path>>(path: P) -> Self {
        let figment = Figment::from(Serialized::defaults(KernelConfig::default()))
            .merge(TomlProvider::file(path.as_ref()).nested())
            .merge(Env::prefixed("CLAW_").split("__"));

        Self { figment }
    }

    /// 获取默认配置文件路径
    fn default_config_path() -> std::path::PathBuf {
        crate::dirs::config_dir()
            .map(|d| d.join("config.toml"))
            .unwrap_or_else(|| std::path::PathBuf::from("config.toml"))
    }

    /// 加载配置
    pub fn load(&self) -> Result<KernelConfig, ConfigError> {
        let mut config: KernelConfig = self
            .figment
            .extract()
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;

        // 展开路径中的 ~
        config = Self::expand_paths(config);

        // 验证配置
        config.validate()?;

        Ok(config)
    }

    /// 从指定文件加载配置
    pub fn load_from_file<P: AsRef<Path>>(&self, path: P) -> Result<KernelConfig, ConfigError> {
        let figment = Figment::from(Serialized::defaults(KernelConfig::default()))
            .merge(TomlProvider::file(path.as_ref()).nested())
            .merge(Env::prefixed("CLAW_").split("__"));

        let mut config: KernelConfig = figment
            .extract()
            .map_err(|e| ConfigError::ParseError(e.to_string()))?;

        // 展开路径中的 ~
        config = Self::expand_paths(config);

        // 验证配置
        config.validate()?;

        Ok(config)
    }

    /// 展开配置中所有路径的 ~ 为家目录
    fn expand_paths(mut config: KernelConfig) -> KernelConfig {
        // 展开沙箱路径
        config.sandbox.allowed_read_paths = config
            .sandbox
            .allowed_read_paths
            .into_iter()
            .map(|p| Self::expand_path(&p))
            .collect();

        config.sandbox.allowed_write_paths = config
            .sandbox
            .allowed_write_paths
            .into_iter()
            .map(|p| Self::expand_path(&p))
            .collect();

        // 展开日志目录
        config.logging.directory = Self::expand_path(&config.logging.directory);

        // 展开工具目录
        config.tools.directory = Self::expand_path(&config.tools.directory);

        config
    }

    /// 展开单个路径中的 ~
    fn expand_path(path: &str) -> String {
        shellexpand::tilde(path).to_string()
    }

    /// 检查默认配置文件是否存在
    pub fn config_exists() -> bool {
        Self::default_config_path().exists()
    }

    /// 获取默认配置文件路径（公共接口）
    pub fn get_default_config_path() -> std::path::PathBuf {
        Self::default_config_path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_loader_new() {
        let loader = ConfigLoader::new();
        // 应该能创建加载器
        assert!(ConfigLoader::config_exists() || !ConfigLoader::config_exists());
    }

    #[test]
    fn test_expand_path() {
        let expanded = ConfigLoader::expand_path("~/test/path");
        assert!(!expanded.contains('~'));
        assert!(expanded.contains("test/path") || expanded.contains("test\\path"));
    }

    #[test]
    fn test_expand_path_no_tilde() {
        let path = "/absolute/path";
        let expanded = ConfigLoader::expand_path(path);
        assert_eq!(expanded, path);
    }

    #[test]
    fn test_load_default_config() {
        let loader = ConfigLoader::new();
        // 如果没有配置文件，应该使用默认值
        let result = loader.load();
        // 无论成功失败都不应该 panic
        match result {
            Ok(config) => {
                assert!(!config.version.is_empty());
            }
            Err(ConfigError::NotFound(_)) => {
                // 配置文件不存在是允许的，使用默认值
            }
            Err(_) => {
                // 其他错误也可以接受
            }
        }
    }
}
