//! Directory management for claw-kernel.

use std::path::PathBuf;

/// Get the configuration directory.
pub fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("claw-kernel"))
}

/// Get the data directory.
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("claw-kernel"))
}

/// Get the cache directory.
pub fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("claw-kernel"))
}

/// Get the tools directory.
pub fn tools_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("tools"))
}

/// Get the scripts directory.
pub fn scripts_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("scripts"))
}

/// Get the logs directory.
pub fn logs_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("logs"))
}

/// Get the agents directory.
pub fn agents_dir() -> Option<PathBuf> {
    data_dir().map(|d| d.join("agents"))
}

/// Get the power key file path.
pub fn power_key_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("power.key"))
}

/// Kernel directory paths.
#[derive(Debug, Clone)]
pub struct KernelDirs {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub log_dir: PathBuf,
    pub agents_dir: PathBuf,
    pub tools_dir: PathBuf,
}

impl KernelDirs {
    /// Create a new KernelDirs instance.
    pub fn new() -> Result<Self, std::io::Error> {
        let config_dir = Self::config_dir()?;
        let data_dir = Self::data_dir()?;

        Ok(Self {
            log_dir: data_dir.join("logs"),
            agents_dir: data_dir.join("agents"),
            tools_dir: data_dir.join("tools"),
            config_dir,
            data_dir,
        })
    }

    /// Get the configuration directory.
    pub fn config_dir() -> Result<PathBuf, std::io::Error> {
        dirs::config_dir()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not find config directory",
                )
            })
            .map(|d| d.join("claw-kernel"))
    }

    /// Get the data directory.
    pub fn data_dir() -> Result<PathBuf, std::io::Error> {
        dirs::data_dir()
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not find data directory",
                )
            })
            .map(|d| d.join("claw-kernel"))
    }

    /// Ensure all directories exist, creating them if necessary.
    pub async fn ensure_all(&self) -> Result<(), std::io::Error> {
        tokio::fs::create_dir_all(&self.config_dir).await?;
        tokio::fs::create_dir_all(&self.data_dir).await?;
        tokio::fs::create_dir_all(&self.log_dir).await?;
        tokio::fs::create_dir_all(&self.agents_dir).await?;
        tokio::fs::create_dir_all(&self.tools_dir).await?;
        Ok(())
    }

    /// Create directories synchronously.
    pub fn ensure_all_sync(&self) -> Result<(), std::io::Error> {
        std::fs::create_dir_all(&self.config_dir)?;
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        std::fs::create_dir_all(&self.agents_dir)?;
        std::fs::create_dir_all(&self.tools_dir)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_kernel_dirs_structure() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let dirs = KernelDirs {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            log_dir: root.join("data/logs"),
            agents_dir: root.join("data/agents"),
            tools_dir: root.join("data/tools"),
        };

        assert_eq!(dirs.log_dir, root.join("data/logs"));
        assert_eq!(dirs.agents_dir, root.join("data/agents"));
        assert_eq!(dirs.tools_dir, root.join("data/tools"));
    }

    #[tokio::test]
    async fn test_ensure_all() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        let dirs = KernelDirs {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            log_dir: root.join("data/logs"),
            agents_dir: root.join("data/agents"),
            tools_dir: root.join("data/tools"),
        };

        dirs.ensure_all().await.unwrap();

        assert!(dirs.config_dir.exists());
        assert!(dirs.log_dir.exists());
        assert!(dirs.agents_dir.exists());
        assert!(dirs.tools_dir.exists());
    }
}
